use std::io;
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratcn::runtime::{FocusState, ModalState};
use ratcn::{InputState, Toast, ToasterState};

mod commands;
mod form;
mod ports;
mod sandbox;

pub use self::commands::PendingCommand;
pub use self::form::{Form, FormField};
pub use self::ports::{PortField, PortForm, PortMapping, PortsView, fetch_ports};
pub use self::sandbox::{Sandbox, SandboxDetails, fetch_sandboxes, inspect_sandbox};

use self::sandbox::push_error;

pub const AGENTS: &[&str] = &[
    "claude",
    "codex",
    "copilot",
    "cursor",
    "devin",
    "docker-agent",
    "droid",
    "kiro",
    "opencode",
    "shell",
];

/// Stable ids for the ratcn modal layers.
pub const CREATE_DIALOG: &str = "create-dialog";
pub const INFO_DIALOG: &str = "info-dialog";
pub const CONFIRM_DIALOG: &str = "confirm-dialog";
pub const PORTS_DIALOG: &str = "ports-dialog";

const CREATE_TOAST: &str = "create-sandbox";

/// Advanced once per event-loop frame while `sbx create` runs in the worker.
pub const SPINNER_FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// How often the dashboard re-reads `sbx ls` to keep running-state dots fresh.
pub const STATUS_REFRESH_INTERVAL: Duration = Duration::from_secs(5);

/// How often an open Info dialog re-collects live guest metrics.
pub const INFO_REFRESH_INTERVAL: Duration = Duration::from_secs(10);

/// Which screen the app is logically on. Derived from the ratcn modal stack,
/// kept as a convenience for status text and tests.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Normal,
    Wizard,
    Info,
    Confirm,
    Ports,
}

// --- app --------------------------------------------------------------------

/// A `sbx create` running on a worker thread. The UI keeps rendering (and
/// ignoring input) until the result arrives on the channel.
struct Creation {
    name: String,
    result: Receiver<io::Result<std::process::Output>>,
}

/// One in-flight metrics collection for an open Info dialog. The guest-side
/// CPU sample sleeps for a second, so this runs off the event-loop thread.
struct InfoRefresh {
    name: String,
    result: Receiver<SandboxDetails>,
}

/// An action waiting for the user to confirm it in the modal dialog. Only the
/// name is captured: the state is checked again when the user confirms, so a
/// periodic status refresh cannot start a stopped sandbox by accident.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PendingAction {
    Start(String),
    Stop(String),
    Remove(String),
}

impl PendingAction {
    fn verb(&self) -> &'static str {
        match self {
            Self::Start(_) => "Start",
            Self::Stop(_) => "Stop",
            Self::Remove(_) => "Remove",
        }
    }
}

pub struct App {
    sandboxes: Vec<Sandbox>,
    /// The row the list cursor is on. Hover and arrow keys move it; the
    /// Actions pane follows `selected`, which a click commits.
    cursor: usize,
    selected: usize,
    list_offset: usize,
    focus: FocusState,
    modals: ModalState,
    form: Form,
    info: Option<SandboxDetails>,
    info_refresh: Option<InfoRefresh>,
    ports: Option<PortsView>,
    port_form: PortForm,
    pending_action: Option<PendingAction>,
    pending_external: Option<PendingCommand>,
    creation: Option<Creation>,
    spinner_index: usize,
    message: String,
    toasts: ToasterState<'static>,
    started: Instant,
    should_quit: bool,
    /// In tests, `perform_action` records instead of running `sbx`.
    #[cfg(test)]
    confirmed_for_test: Option<PendingAction>,
    /// In tests, publishing records the command instead of running `sbx`.
    #[cfg(test)]
    published_for_test: Option<String>,
    /// In tests, unpublishing records the command instead of running `sbx`.
    #[cfg(test)]
    unpublished_for_test: Option<String>,
}

impl App {
    pub fn new() -> Self {
        let mut app = Self {
            sandboxes: Vec::new(),
            cursor: 0,
            selected: 0,
            list_offset: 0,
            focus: FocusState::default(),
            modals: ModalState::default(),
            form: Form::new(),
            info: None,
            info_refresh: None,
            ports: None,
            port_form: PortForm::new(),
            pending_action: None,
            pending_external: None,
            creation: None,
            spinner_index: 0,
            message: String::new(),
            toasts: ToasterState::new(),
            started: Instant::now(),
            should_quit: false,
            #[cfg(test)]
            confirmed_for_test: None,
            #[cfg(test)]
            published_for_test: None,
            #[cfg(test)]
            unpublished_for_test: None,
        };
        app.refresh_sandboxes();
        app
    }

    pub fn should_quit(&self) -> bool {
        self.should_quit
    }

    pub fn quit(&mut self) {
        self.should_quit = true;
    }

    /// Logical screen, derived from the modal stack so the two can never
    /// disagree about which layer is active.
    pub fn mode(&self) -> Mode {
        if self.modals.is_open(CREATE_DIALOG) {
            Mode::Wizard
        } else if self.modals.is_open(INFO_DIALOG) {
            Mode::Info
        } else if self.modals.is_open(CONFIRM_DIALOG) {
            Mode::Confirm
        } else if self.modals.is_open(PORTS_DIALOG) {
            Mode::Ports
        } else {
            Mode::Normal
        }
    }

    /// The action the confirmation dialog is asking about, if one is open.
    pub fn pending_action(&self) -> Option<&PendingAction> {
        self.pending_action.as_ref()
    }

    pub fn focus_ref(&self) -> &FocusState {
        &self.focus
    }

    pub fn focus_mut(&mut self) -> &mut FocusState {
        &mut self.focus
    }

    pub fn modals_ref(&self) -> &ModalState {
        &self.modals
    }

    pub fn form(&self) -> &Form {
        &self.form
    }

    pub fn set_form_state(&mut self, field: FormField, state: InputState) {
        match field {
            FormField::Name => self.form.name = state,
            FormField::RootSize => self.form.root_size = state,
            FormField::DockerSize => self.form.docker_size = state,
            FormField::Cpus => self.form.cpus = state,
            FormField::Memory => self.form.memory = state,
        }
    }

    pub fn set_agent_index(&mut self, index: usize) {
        self.form.set_agent_index(index);
    }

    pub fn set_agent_focus(&mut self, index: usize) {
        self.form.set_agent_focus(index);
    }

    pub fn set_agent_open(&mut self, open: bool) {
        self.form.set_agent_open(open);
    }

    pub fn info(&self) -> Option<&SandboxDetails> {
        self.info.as_ref()
    }

    pub fn ports(&self) -> Option<&PortsView> {
        self.ports.as_ref()
    }

    pub fn port_form(&self) -> &PortForm {
        &self.port_form
    }

    pub fn port_invalid(&self, field: PortField) -> bool {
        self.port_form.invalid(field)
    }

    pub fn set_port_state(&mut self, field: PortField, state: InputState) {
        match field {
            PortField::Host => self.port_form.host = state,
            PortField::Sandbox => self.port_form.sandbox = state,
        }
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn now(&self) -> Duration {
        self.started.elapsed()
    }

    pub fn toasts(&self) -> &ToasterState<'static> {
        &self.toasts
    }

    pub fn prune_toasts(&mut self, now: Duration) -> bool {
        self.toasts.prune_expired(now)
    }

    fn notify(&mut self, toast: Toast<'static>, detail: Option<String>) {
        self.message = match detail {
            Some(ref detail) if !detail.is_empty() => format!("{}: {detail}", toast.title()),
            _ => toast.title().to_owned(),
        };
        let toast = match detail {
            Some(detail) if !detail.is_empty() => toast.with_description(detail),
            _ => toast,
        };
        let now = self.now();
        self.toasts.push(toast, now);
    }

    pub fn notify_info(&mut self, title: impl Into<String>, detail: Option<String>) {
        self.notify(Toast::info(title.into()), detail);
    }

    pub fn notify_success(&mut self, title: impl Into<String>, detail: Option<String>) {
        self.notify(Toast::success(title.into()), detail);
    }

    pub fn notify_error(&mut self, title: impl Into<String>, detail: impl Into<String>) {
        let detail = detail.into();
        self.notify(Toast::error(title.into()), Some(detail));
    }

    /// Taken by the event loop when an Exec button was clicked. The terminal
    /// is suspended before the returned command is run.
    pub fn take_external_command(&mut self) -> Option<PendingCommand> {
        self.pending_external.take()
    }

    /// True from the moment the create button is pressed until the worker
    /// thread reports back. Input is ignored during this window.
    pub fn is_creating(&self) -> bool {
        self.creation.is_some()
    }

    pub fn creating_name(&self) -> Option<&str> {
        self.creation
            .as_ref()
            .map(|creation| creation.name.as_str())
    }

    pub fn spinner(&self) -> &'static str {
        SPINNER_FRAMES[self.spinner_index % SPINNER_FRAMES.len()]
    }

    /// Fraction for the themed progress bar shown in the create dialog.
    pub fn progress(&self) -> f64 {
        (self.spinner_index % SPINNER_FRAMES.len()) as f64 / (SPINNER_FRAMES.len() - 1) as f64
    }

    /// Advance the spinner once per rendered frame, but only while a sandbox
    /// is actually being created.
    pub fn tick(&mut self) {
        if self.is_creating() {
            self.spinner_index = self.spinner_index.wrapping_add(1);
        }
    }

    /// Pick up a finished `sbx create` without blocking the event loop.
    pub fn poll_creation(&mut self) {
        let (name, result) = {
            let Some(creation) = self.creation.as_ref() else {
                return;
            };
            let result = match creation.result.try_recv() {
                Ok(result) => Some(result),
                Err(mpsc::TryRecvError::Disconnected) => {
                    Some(Err(io::Error::other("creation worker stopped")))
                }
                Err(mpsc::TryRecvError::Empty) => None,
            };
            (creation.name.clone(), result)
        };
        if let Some(result) = result {
            self.creation = None;
            self.finish_creation(result, &name);
        }
    }

    /// Pick up a finished metrics collection without blocking the event loop.
    /// Results for a sandbox that is no longer shown are dropped.
    pub fn poll_info(&mut self) {
        let Some(refresh) = self.info_refresh.as_ref() else {
            return;
        };
        let result = match refresh.result.try_recv() {
            Ok(details) => Some(details),
            Err(mpsc::TryRecvError::Disconnected) => None,
            Err(mpsc::TryRecvError::Empty) => return,
        };
        let name = refresh.name.clone();
        self.info_refresh = None;
        if self.info.as_ref().is_some_and(|info| info.name == name) {
            match result {
                Some(details) => self.info = Some(details),
                None => {
                    if let Some(info) = self.info.as_mut() {
                        push_error(info, "Metrics worker stopped.".to_owned());
                    }
                }
            }
        }
    }

    /// True while the Info dialog is waiting for its metrics worker.
    pub fn is_refreshing_info(&self) -> bool {
        self.info_refresh.is_some()
    }

    /// Start a new metrics collection for the sandbox shown in Info. Called
    /// when the dialog opens and on the ten-second ticker; overlapping runs are
    /// skipped so slow guest commands cannot pile up.
    pub fn refresh_info(&mut self) {
        if self.is_refreshing_info() {
            return;
        }
        let Some(name) = self.info.as_ref().map(|info| info.name.clone()) else {
            return;
        };
        let Some(sandbox) = self
            .sandboxes
            .iter()
            .find(|sandbox| sandbox.name == name)
            .cloned()
        else {
            return;
        };
        self.start_info_refresh(&sandbox);
    }

    fn start_info_refresh(&mut self, sandbox: &Sandbox) {
        if self.is_refreshing_info() {
            return;
        }
        let name = sandbox.name.clone();
        let sandbox = sandbox.clone();
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let _ = sender.send(inspect_sandbox(&sandbox));
        });
        self.info_refresh = Some(InfoRefresh {
            name,
            result: receiver,
        });
    }

    pub fn sandboxes(&self) -> &[Sandbox] {
        &self.sandboxes
    }

    /// The row the list cursor is on. Kept apart from the selected sandbox so
    /// hovering a row does not silently retarget the Actions pane.
    pub fn focused_name(&self) -> Option<String> {
        self.sandboxes.get(self.cursor).map(|s| s.name.clone())
    }

    pub fn selected_name(&self) -> Option<String> {
        self.sandboxes.get(self.selected).map(|s| s.name.clone())
    }

    pub fn selected_sandbox(&self) -> Option<&Sandbox> {
        self.sandboxes.get(self.selected)
    }

    pub fn list_offset(&self) -> usize {
        self.list_offset
    }

    pub fn set_list_offset(&mut self, offset: usize) {
        self.list_offset = offset.min(self.sandboxes.len().saturating_sub(1));
    }

    /// Move the cursor (and the visible window) without changing which sandbox
    /// the Actions pane acts on. Keyboard navigation and hover use this.
    pub fn focus_list_item(&mut self, name: &str, offset: usize) {
        if let Some(index) = self.sandboxes.iter().position(|s| s.name == name) {
            self.cursor = index;
        }
        self.set_list_offset(offset);
    }

    /// Commit a sandbox as the target of the Actions pane and put keyboard
    /// focus on the list so Tab continues into those actions.
    pub fn focus_sandbox(&mut self, name: &str) {
        self.select_named(name);
        *self.focus_mut() = FocusState::intent(["sandbox-list"]);
    }

    pub fn select_named(&mut self, name: &str) {
        if let Some(index) = self.sandboxes.iter().position(|s| s.name == name) {
            self.selected = index;
            self.cursor = index;
        }
    }

    /// Keys that are not claimed by a ratcn component still act as global
    /// shortcuts. Creation is modal: only Ctrl+C gets through.
    pub fn handle_global_key(&mut self, key: KeyEvent) {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.should_quit = true;
            return;
        }
        if self.is_creating() || self.mode() != Mode::Normal {
            return;
        }

        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => self.quit(),
            KeyCode::Char('r') | KeyCode::Char('R') => {
                self.refresh_sandboxes();
                self.notify_success("Sandbox list refreshed", None);
            }
            KeyCode::Char('n') | KeyCode::Char('N') => self.open_wizard(),
            KeyCode::Char('s') | KeyCode::Char('S') => self.toggle_selected(),
            KeyCode::Char('x') | KeyCode::Char('X') => self.exec_selected(),
            KeyCode::Char('d') | KeyCode::Char('D') => self.remove_selected(),
            KeyCode::Char('i') | KeyCode::Char('I') => self.open_selected_info(),
            KeyCode::Char('p') | KeyCode::Char('P') => self.open_ports_selected(),
            _ => {}
        }
    }

    pub fn open_wizard(&mut self) {
        self.form = Form::new();
        let _ = self.modals.open(CREATE_DIALOG, &mut self.focus);
    }

    /// Cancel via the dialog's close button or Escape.
    pub fn cancel_wizard(&mut self) {
        self.close_wizard();
        self.notify_info("Create sandbox cancelled.", None);
    }

    fn close_wizard(&mut self) {
        if self
            .modals
            .top()
            .is_some_and(|id| id.as_str() == CREATE_DIALOG)
        {
            self.modals.close(&mut self.focus);
        }
        self.form = Form::new();
    }

    pub fn create_from_form(&mut self) {
        if let Some((field, error)) = self.form.first_error() {
            self.notify_error(
                "Invalid sandbox setting",
                format!("{}: {error}", field.label()),
            );
            return;
        }
        let pending = self.form.pending_command();
        let name = self.form.name.value().trim().to_owned();
        self.start_creation(pending, &name);
    }

    pub fn open_info(&mut self, name: &str) {
        let Some(sandbox) = self.sandboxes.iter().find(|s| s.name == name).cloned() else {
            return;
        };
        self.select_named(name);
        // Show the known values immediately; the guest-side metrics arrive on
        // a worker so the one-second CPU sample never freezes the TUI.
        self.info = Some(SandboxDetails {
            name: sandbox.name.clone(),
            status: sandbox.status.clone(),
            ..SandboxDetails::default()
        });
        self.info_refresh = None;
        self.start_info_refresh(&sandbox);
        let _ = self.modals.open(INFO_DIALOG, &mut self.focus);
    }

    pub fn close_info(&mut self) {
        if self
            .modals
            .top()
            .is_some_and(|id| id.as_str() == INFO_DIALOG)
        {
            self.modals.close(&mut self.focus);
        }
        // Invalidate any worker still running: its result must not land in a
        // dialog opened later for a different sandbox.
        self.info = None;
        self.info_refresh = None;
    }

    /// Show the published ports of the sandbox the Actions pane targets.
    pub fn open_ports_selected(&mut self) {
        if let Some(name) = self.selected_name()
            && self.is_running(&name)
        {
            self.open_ports(&name);
        }
    }

    /// Open (or refill) the ports dialog. Listing ports is a quick local `sbx`
    /// call, so the table is ready on the first frame; a failure is shown in
    /// the dialog instead of blocking it.
    pub fn open_ports(&mut self, name: &str) {
        if !self.is_running(name) {
            return;
        }
        self.select_named(name);
        self.port_form = PortForm::new();
        let (ports, error) = match fetch_ports(name) {
            Ok(ports) => (ports, None),
            Err(error) => (Vec::new(), Some(error)),
        };
        self.ports = Some(PortsView {
            name: name.to_owned(),
            ports,
            error,
        });
        let _ = self.modals.open(PORTS_DIALOG, &mut self.focus);
    }

    pub fn close_ports(&mut self) {
        if self
            .modals
            .top()
            .is_some_and(|id| id.as_str() == PORTS_DIALOG)
        {
            self.modals.close(&mut self.focus);
        }
        self.ports = None;
    }

    /// Re-read the table after a publish, keeping the dialog open.
    #[cfg_attr(test, allow(dead_code))]
    fn refresh_ports(&mut self) {
        let Some(view) = self.ports.as_mut() else {
            return;
        };
        let name = view.name.clone();
        match fetch_ports(&name) {
            Ok(ports) => {
                view.ports = ports;
                view.error = None;
            }
            Err(error) => view.error = Some(error),
        }
    }

    /// The **Add port** button: validate both fields, then run
    /// `sbx port publish <name> <host>:<sandbox>` and refresh the table.
    pub fn publish_port(&mut self) {
        if self.mode() != Mode::Ports {
            return;
        }
        let Some(name) = self.ports.as_ref().map(|view| view.name.clone()) else {
            return;
        };
        if let Some(error) = self.port_form.first_error() {
            if let Some(view) = self.ports.as_mut() {
                view.error = Some(error.clone());
            }
            self.notify_error("Invalid port", error);
            return;
        }

        let pending = self.port_form.pending_command(&name);

        #[cfg(test)]
        {
            self.published_for_test = Some(pending.preview());
        }

        #[cfg(not(test))]
        let host = self.port_form.port(PortField::Host).unwrap_or(0);
        #[cfg(not(test))]
        let sandbox = self.port_form.port(PortField::Sandbox).unwrap_or(0);

        #[cfg(not(test))]
        match pending.run() {
            Ok(output) if output.status.success() => {
                self.refresh_ports();
                self.notify_success(format!("Published {host}:{sandbox} for '{name}'."), None);
            }
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                let detail = stderr
                    .lines()
                    .find(|line| !line.trim().is_empty())
                    .unwrap_or("sbx port publish failed")
                    .to_owned();
                if let Some(view) = self.ports.as_mut() {
                    view.error = Some(detail.clone());
                }
                self.notify_error("Could not publish port", detail);
            }
            Err(error) => {
                let detail = error.to_string();
                if let Some(view) = self.ports.as_mut() {
                    view.error = Some(detail.clone());
                }
                self.notify_error("Could not publish port", detail);
            }
        }
    }

    /// The unpublish button on a table row: run
    /// `sbx port unpublish <name> <host>:<sandbox>` and refresh the table.
    pub fn unpublish_port(&mut self, host: u16, sandbox: u16) {
        if self.mode() != Mode::Ports {
            return;
        }
        let Some(name) = self.ports.as_ref().map(|view| view.name.clone()) else {
            return;
        };
        let pending = PendingCommand::new(
            "sbx",
            &["port", "unpublish", &name, &format!("{host}:{sandbox}")],
        );

        #[cfg(test)]
        {
            self.unpublished_for_test = Some(pending.preview());
        }

        #[cfg(not(test))]
        match pending.run() {
            Ok(output) if output.status.success() => {
                self.refresh_ports();
                self.notify_success(format!("Unpublished {host}:{sandbox} for '{name}'."), None);
            }
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                let detail = stderr
                    .lines()
                    .find(|line| !line.trim().is_empty())
                    .unwrap_or("sbx port unpublish failed")
                    .to_owned();
                if let Some(view) = self.ports.as_mut() {
                    view.error = Some(detail.clone());
                }
                self.notify_error("Could not unpublish port", detail);
            }
            Err(error) => {
                let detail = error.to_string();
                if let Some(view) = self.ports.as_mut() {
                    view.error = Some(detail.clone());
                }
                self.notify_error("Could not unpublish port", detail);
            }
        }
    }

    pub fn toggle_selected(&mut self) {
        let Some(name) = self.selected_name() else {
            return;
        };
        if self.is_running(&name) {
            self.request_action(PendingAction::Stop(name));
        } else {
            self.request_action(PendingAction::Start(name));
        }
    }

    /// The Actions pane only offers Start while a sandbox is stopped.
    pub fn start_selected(&mut self) {
        if let Some(name) = self.selected_name()
            && !self.is_running(&name)
        {
            self.request_action(PendingAction::Start(name));
        }
    }

    /// The Actions pane only offers Stop while a sandbox is running.
    pub fn stop_selected(&mut self) {
        if let Some(name) = self.selected_name()
            && self.is_running(&name)
        {
            self.request_action(PendingAction::Stop(name));
        }
    }

    /// Start/Stop/Remove never run directly from a button or shortcut: they
    /// first park here so the user can confirm the action in a dialog.
    fn request_action(&mut self, action: PendingAction) {
        if self.mode() != Mode::Normal {
            return;
        }
        self.pending_action = Some(action);
        let _ = self.modals.open(CONFIRM_DIALOG, &mut self.focus);
    }

    /// The dialog's Cancel button or Escape.
    pub fn cancel_action(&mut self) {
        let Some(action) = self.pending_action.take() else {
            return;
        };
        if self
            .modals
            .top()
            .is_some_and(|id| id.as_str() == CONFIRM_DIALOG)
        {
            self.modals.close(&mut self.focus);
        }
        self.notify_info(format!("{} cancelled.", action.verb()), None);
    }

    /// The dialog's confirm button. Recheck the sandbox state before acting:
    /// the five-second status ticker may have changed it while the dialog was
    /// open, and a removed sandbox should never be acted on.
    pub fn confirm_action(&mut self) {
        let Some(action) = self.pending_action.take() else {
            return;
        };
        if self
            .modals
            .top()
            .is_some_and(|id| id.as_str() == CONFIRM_DIALOG)
        {
            self.modals.close(&mut self.focus);
        }
        let should_be_running = matches!(&action, PendingAction::Stop(_));
        let still_valid = match &action {
            PendingAction::Start(name) | PendingAction::Stop(name) => {
                self.has_sandbox(name) && self.is_running(name) == should_be_running
            }
            PendingAction::Remove(name) => self.has_sandbox(name),
        };
        if still_valid {
            self.perform_action(action);
        } else {
            self.notify_info("Sandbox state changed; action cancelled.", None);
        }
    }

    fn perform_action(&mut self, action: PendingAction) {
        #[cfg(test)]
        {
            self.confirmed_for_test = Some(action);
        }
        #[cfg(not(test))]
        match action {
            PendingAction::Start(name) => self.start_sandbox(&name),
            PendingAction::Stop(name) => self.stop_sandbox(&name),
            PendingAction::Remove(name) => self.remove_sandbox(&name),
        }
    }

    #[cfg_attr(test, allow(dead_code))]
    fn start_sandbox(&mut self, name: &str) {
        let command = Self::start_command(name);
        self.run_sbx(command, format!("Sandbox '{name}' started."));
    }

    /// `sbx` has no standalone start verb. `run --name` re-attaches to the
    /// existing sandbox and starts it; `-d` leaves it running detached.
    fn start_command(name: &str) -> PendingCommand {
        PendingCommand::new("sbx", &["run", "-d", "--name", name])
    }

    #[cfg_attr(test, allow(dead_code))]
    fn stop_sandbox(&mut self, name: &str) {
        let command = PendingCommand::new("sbx", &["stop", name]);
        self.run_sbx(command, format!("Sandbox '{name}' stopped."));
    }

    pub fn exec_selected(&mut self) {
        let Some(name) = self.selected_name() else {
            return;
        };
        if !self.is_running(&name) {
            return;
        }
        self.pending_external = Some(PendingCommand::new("sbx", &["exec", "-it", &name, "bash"]));
        self.notify_info(format!("Opening external terminal for '{name}'..."), None);
    }

    pub fn remove_selected(&mut self) {
        if let Some(name) = self.selected_name() {
            self.request_action(PendingAction::Remove(name));
        }
    }

    #[cfg_attr(test, allow(dead_code))]
    fn remove_sandbox(&mut self, name: &str) {
        // The TUI has no TTY to answer sbx's confirmation prompt, so removal
        // is explicit and non-interactive here.
        let command = PendingCommand::new("sbx", &["remove", name, "--force"]);
        self.run_sbx(command, format!("Sandbox '{name}' removed."));
    }

    pub fn open_selected_info(&mut self) {
        if let Some(name) = self.selected_name()
            && self.is_running(&name)
        {
            self.open_info(&name);
        }
    }

    fn is_running(&self, name: &str) -> bool {
        self.sandboxes
            .iter()
            .find(|sandbox| sandbox.name == name)
            .is_some_and(Sandbox::is_running)
    }

    fn has_sandbox(&self, name: &str) -> bool {
        self.sandboxes.iter().any(|sandbox| sandbox.name == name)
    }

    fn start_creation(&mut self, pending: PendingCommand, name: &str) {
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let _ = sender.send(pending.run());
        });
        self.creation = Some(Creation {
            name: name.to_owned(),
            result: receiver,
        });
        self.spinner_index = 0;
        let now = self.now();
        self.toasts.push(
            Toast::loading(format!("Creating sandbox '{name}'"))
                .with_id(CREATE_TOAST)
                .persistent(),
            now,
        );
        self.message = format!("Creating sandbox '{name}'...");
    }

    pub fn refresh_sandboxes(&mut self) {
        match fetch_sandboxes() {
            Ok(sandboxes) => self.apply_statuses(sandboxes),
            Err(error) => self.notify_error("Could not list sandboxes", error),
        }
    }

    /// Periodic tick: keep running-state dots (and an open Info modal's status
    /// line) current without re-running the in-guest metrics commands.
    pub fn refresh_statuses(&mut self) {
        if let Ok(sandboxes) = fetch_sandboxes() {
            self.apply_statuses(sandboxes);
        }
    }

    fn apply_statuses(&mut self, sandboxes: Vec<Sandbox>) {
        if let Some(info) = self.info.as_mut()
            && let Some(fresh) = sandboxes.iter().find(|sandbox| sandbox.name == info.name)
        {
            info.status = fresh.status.clone();
        }
        self.sandboxes = sandboxes;
        let last = self.sandboxes.len().saturating_sub(1);
        self.cursor = self.cursor.min(last);
        self.selected = self.selected.min(last);
        self.list_offset = self.list_offset.min(last);
    }

    #[cfg_attr(test, allow(dead_code))]
    fn run_sbx(&mut self, pending: PendingCommand, success: String) {
        self.handle_sbx_result(pending.run(), success);
    }

    fn handle_sbx_result(&mut self, result: io::Result<std::process::Output>, success: String) {
        match result {
            Ok(output) if output.status.success() => {
                self.refresh_statuses();
                self.notify_success(success, None);
            }
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                let detail = stderr
                    .lines()
                    .find(|line| !line.trim().is_empty())
                    .unwrap_or("sbx command failed")
                    .to_owned();
                self.notify_error("Could not run sbx", detail);
            }
            Err(error) => self.notify_error("Could not run sbx", error.to_string()),
        }
    }

    fn finish_creation(&mut self, result: io::Result<std::process::Output>, name: &str) {
        let succeeded = matches!(&result, Ok(output) if output.status.success());
        let _ = self.toasts.dismiss(CREATE_TOAST);
        self.handle_sbx_result(result, format!("Sandbox '{name}' created."));
        if succeeded {
            self.close_wizard();
        }
        // On failure the dialog stays open so the settings can be corrected or
        // the create can be retried.
    }
}

#[cfg(test)]
impl App {
    /// Put the app into the same state `start_creation` does, without spawning
    /// a real `sbx` process. The sender is dropped, so `poll_creation` observes
    /// a disconnected worker.
    pub fn begin_creation_for_test(&mut self, name: &str) {
        let (sender, receiver) = mpsc::channel();
        drop(sender);
        self.creation = Some(Creation {
            name: name.to_owned(),
            result: receiver,
        });
        self.spinner_index = 0;
        self.message = format!("Creating sandbox '{name}'...");
        self.toasts.push(
            Toast::loading(format!("Creating sandbox '{name}'"))
                .with_id(CREATE_TOAST)
                .persistent(),
            self.now(),
        );
        let _ = self.modals.open(CREATE_DIALOG, &mut self.focus);
    }

    pub fn complete_creation_for_test(
        &mut self,
        name: &str,
        result: io::Result<std::process::Output>,
    ) {
        self.creation = None;
        self.finish_creation(result, name);
    }

    pub fn with_sandboxes(names: &[&str]) -> Self {
        Self::with_sandbox_statuses(
            &names
                .iter()
                .map(|name| (*name, "running"))
                .collect::<Vec<_>>(),
        )
    }

    pub fn with_sandbox_statuses(entries: &[(&str, &str)]) -> Self {
        let mut app = Self::new();
        app.sandboxes = entries
            .iter()
            .map(|(name, status)| Sandbox::new((*name).to_owned(), (*status).to_owned()))
            .collect();
        app
    }

    pub fn show_info_for_test(&mut self, details: SandboxDetails) {
        self.info = Some(details);
        let _ = self.modals.open(INFO_DIALOG, &mut self.focus);
    }

    /// Open the ports dialog with a known table, without calling `sbx`.
    pub fn show_ports_for_test(&mut self, name: &str, ports: Vec<PortMapping>) {
        self.select_named(name);
        self.ports = Some(PortsView {
            name: name.to_owned(),
            ports,
            error: None,
        });
        let _ = self.modals.open(PORTS_DIALOG, &mut self.focus);
    }

    /// The command the ports dialog handed to `perform_action`-style test hook.
    pub fn take_published_for_test(&mut self) -> Option<String> {
        self.published_for_test.take()
    }

    /// The command a row's unpublish button handed to the test hook.
    pub fn take_unpublished_for_test(&mut self) -> Option<String> {
        self.unpublished_for_test.take()
    }

    /// Queue a fake metrics result so `poll_info` can be tested without
    /// running `sbx exec` inside the guest.
    pub fn queue_info_result_for_test(&mut self, name: &str, details: SandboxDetails) {
        let (sender, receiver) = mpsc::channel();
        sender.send(details).expect("test channel");
        self.info_refresh = Some(InfoRefresh {
            name: name.to_owned(),
            result: receiver,
        });
    }

    /// The action a confirmed dialog handed to `perform_action`. Tests use
    /// this instead of letting the app shell out to `sbx`.
    pub fn take_confirmed_for_test(&mut self) -> Option<PendingAction> {
        self.confirmed_for_test.take()
    }
}

#[cfg(test)]
mod tests;
