//! Application state and the small, shared operations around it. Feature
//! workflows live in child modules; they can still reach `App`'s private
//! fields, so no getter churn is needed to split them out.

use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratcn::runtime::{FocusState, ModalState};
use ratcn::{InputState, Toast, ToasterState};

mod actions;
mod commands;
mod dialogs;
mod form;
mod ports;
mod ports_flow;
mod sandbox;
mod secret_form;
mod secrets;
mod service_catalog;
#[cfg(test)]
pub(crate) mod test_support;
mod workers;
mod workspace;

pub use self::actions::PendingAction;
pub use self::commands::PendingCommand;
pub use self::form::{Form, FormField, WorkspaceMode};
pub use self::ports::{PortField, PortForm, PortMapping, PortsView};
pub use self::sandbox::{
    HostMount, MountMode, Sandbox, SandboxDetails, fetch_sandboxes, inspect_sandbox,
};
// `GLOBAL_SCOPE` and `fetch_secrets` are public entry points for callers that
// don't cache sandboxes; the dashboard itself uses the cached variant.
#[allow(unused_imports)]
pub use self::secret_form::{GLOBAL_SCOPE, SERVICES, SecretForm, is_global_scope};
#[allow(unused_imports)]
pub use self::secrets::{Secret, fetch_secrets, fetch_secrets_for};
pub use self::service_catalog::service_info;
pub use self::workspace::{WORKSPACE_PARENT, WorkspacePicker};

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
pub const WORKSPACE_DIALOG: &str = "workspace-dialog";
pub const SET_SECRET_DIALOG: &str = "set-secret-dialog";

/// Stable id for the tab row, used as a focus target when the visible tab changes.
pub const TAB_ROW: &str = "tab-row";

/// The dashboard's top-level tabs. The tab row owns the choice; each tab
/// decides which body section is visible below it.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum DashboardTab {
    #[default]
    Sandboxes,
    Secrets,
}

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
    /// The folder picker sits above the create wizard.
    Workspace,
    SetSecret,
}

pub struct App {
    tab: DashboardTab,
    sandboxes: Vec<Sandbox>,
    secrets: Vec<Secret>,
    /// The row the list cursor is on. Hover and arrow keys move it; the
    /// Actions pane follows `selected`, which a click commits.
    cursor: usize,
    selected: usize,
    list_offset: usize,
    focus: FocusState,
    modals: ModalState,
    form: Form,
    info: Option<SandboxDetails>,
    /// A secret row shown in the same Info dialog as sandbox details.
    secret_info: Option<Secret>,
    info_refresh: Option<workers::InfoRefresh>,
    ports: Option<PortsView>,
    port_form: PortForm,
    workspace_picker: Option<WorkspacePicker>,
    secret_form: Option<SecretForm>,
    pending_action: Option<PendingAction>,
    pending_external: Option<PendingCommand>,
    creation: Option<workers::Creation>,
    spinner_index: usize,
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
    /// In tests, deleting a secret records the command instead of running `sbx`.
    #[cfg(test)]
    deleted_for_test: Option<String>,
    /// In tests, setting a secret records the command instead of running `sbx`.
    #[cfg(test)]
    set_for_test: Option<String>,
    /// In tests, the answer to the "secret already exists" lookup. `None` means
    /// no existing secret, so the usual set flow keeps working unchanged.
    #[cfg(test)]
    secret_exists_for_test: Option<bool>,
}

impl App {
    pub fn new() -> Self {
        let mut app = Self {
            tab: DashboardTab::default(),
            sandboxes: Vec::new(),
            secrets: Vec::new(),
            cursor: 0,
            selected: 0,
            list_offset: 0,
            focus: FocusState::default(),
            modals: ModalState::default(),
            form: Form::new(),
            info: None,
            secret_info: None,
            info_refresh: None,
            ports: None,
            port_form: PortForm::new(),
            workspace_picker: None,
            secret_form: None,
            pending_action: None,
            pending_external: None,
            creation: None,
            spinner_index: 0,
            toasts: ToasterState::new(),
            started: Instant::now(),
            should_quit: false,
            #[cfg(test)]
            confirmed_for_test: None,
            #[cfg(test)]
            published_for_test: None,
            #[cfg(test)]
            unpublished_for_test: None,
            #[cfg(test)]
            deleted_for_test: None,
            #[cfg(test)]
            set_for_test: None,
            #[cfg(test)]
            secret_exists_for_test: None,
        };
        app.refresh_sandboxes();
        app.refresh_secrets();
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
        if self.modals.is_open(WORKSPACE_DIALOG) {
            Mode::Workspace
        } else if self.modals.is_open(CREATE_DIALOG) {
            Mode::Wizard
        } else if self.modals.is_open(INFO_DIALOG) {
            Mode::Info
        } else if self.modals.is_open(CONFIRM_DIALOG) {
            Mode::Confirm
        } else if self.modals.is_open(PORTS_DIALOG) {
            Mode::Ports
        } else if self.modals.is_open(SET_SECRET_DIALOG) {
            Mode::SetSecret
        } else {
            Mode::Normal
        }
    }

    pub fn active_tab(&self) -> DashboardTab {
        self.tab
    }

    /// Switch tabs and park focus on the row, since controls belonging to the
    /// previous tab are no longer declared in the frame.
    pub fn select_tab(&mut self, tab: DashboardTab) {
        self.tab = tab;
        *self.focus_mut() = FocusState::intent([TAB_ROW]);
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

    pub fn set_add_workspace(&mut self, enabled: bool) {
        self.form.set_add_workspace(enabled);
    }

    pub fn set_workspace_mode(&mut self, index: usize) {
        self.form.set_workspace_mode(index);
    }

    pub fn info(&self) -> Option<&SandboxDetails> {
        self.info.as_ref()
    }

    /// The secret row the Info dialog is showing, if any.
    pub fn secret_info(&self) -> Option<&Secret> {
        self.secret_info.as_ref()
    }

    pub fn ports(&self) -> Option<&PortsView> {
        self.ports.as_ref()
    }

    pub fn port_form(&self) -> &PortForm {
        &self.port_form
    }

    pub fn workspace_picker(&self) -> Option<&WorkspacePicker> {
        self.workspace_picker.as_ref()
    }

    pub fn secret_form(&self) -> Option<&SecretForm> {
        self.secret_form.as_ref()
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

    /// Text of the newest toast. Tests use this to check what the user was
    /// told; the UI itself shows notifications only as toasts.
    #[cfg(test)]
    pub fn message(&self) -> String {
        self.toasts()
            .entries()
            .last()
            .map(|entry| {
                let toast = entry.toast();
                match toast.description() {
                    Some(detail) if !detail.is_empty() => {
                        format!("{}: {detail}", toast.title())
                    }
                    _ => toast.title().to_owned(),
                }
            })
            .unwrap_or_default()
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

    pub fn sandboxes(&self) -> &[Sandbox] {
        &self.sandboxes
    }

    pub fn secrets(&self) -> &[Secret] {
        &self.secrets
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
        // These shortcuts belong to the sandbox tab; the Secrets tab uses its
        // own buttons instead.
        if self.active_tab() != DashboardTab::Sandboxes {
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

    pub fn refresh_sandboxes(&mut self) {
        match fetch_sandboxes() {
            Ok(sandboxes) => self.apply_statuses(sandboxes),
            Err(error) => self.notify_error("Could not list sandboxes", error),
        }
    }

    pub fn refresh_secrets(&mut self) {
        match fetch_secrets_for(&self.sandboxes) {
            Ok(secrets) => self.secrets = secrets,
            Err(error) => self.notify_error("Could not list secrets", error),
        }
    }

    /// A row's Delete button only asks for confirmation; the CLI runs once the
    /// dialog is confirmed.
    pub fn delete_secret(&mut self, name: &str, scope: &str) {
        self.request_action(PendingAction::DeleteSecret(
            name.to_owned(),
            scope.to_owned(),
        ));
    }

    /// The row's scope is the sandbox name; `(global)` secrets omit `--sandbox`.
    /// `-f` skips the CLI's own prompt, which cannot be answered while the TUI
    /// owns the terminal.
    fn delete_secret_command(name: &str, scope: &str) -> PendingCommand {
        let mut args = vec!["secrets", "rm", name];
        let scoped = (!is_global_scope(scope)).then_some(["--sandbox", scope]);
        if let Some([flag, value]) = scoped.as_ref() {
            args.push(flag);
            args.push(value);
        }
        args.push("-f");
        PendingCommand::new("sbx", &args)
    }

    /// Run a confirmed deletion, then refresh the table the row came from.
    #[cfg_attr(test, allow(dead_code))]
    fn delete_secret_now(&mut self, name: &str, scope: &str) {
        let pending = Self::delete_secret_command(name, scope);

        #[cfg(test)]
        {
            self.deleted_for_test = Some(pending.preview());
        }

        #[cfg(not(test))]
        {
            let detail = match pending.run() {
                Ok(output) if output.status.success() => {
                    self.refresh_secrets();
                    self.notify_success(format!("Deleted secret '{name}'"), None);
                    return;
                }
                Ok(output) => commands::stderr_detail(&output, "sbx secrets rm failed"),
                Err(error) => error.to_string(),
            };
            self.notify_error("Could not delete secret", detail);
        }
    }

    /// Periodic tick: keep running-state dots (and an open Info modal's status
    /// line) current without re-running the in-guest metrics commands.
    pub fn refresh_statuses(&mut self) {
        if let Ok(sandboxes) = fetch_sandboxes() {
            self.apply_statuses(sandboxes);
        }
    }

    pub(super) fn apply_statuses(&mut self, sandboxes: Vec<Sandbox>) {
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
}

#[cfg(test)]
mod tests;
