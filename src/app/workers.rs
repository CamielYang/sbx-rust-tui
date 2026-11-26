//! Background work: sandbox creation, guest-side metrics, and every `sbx`
//! command triggered by a button. All of them run off the event-loop thread so
//! a slow CLI call never delays the next keypress or click.

use std::io;
use std::process::Output;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use ratcn::Toast;

use super::commands::stderr_detail;
use super::{
    App, PortMapping, Sandbox, SandboxDetails, Secret, inspect_sandbox, sandbox::push_error,
};

pub(super) const CREATE_TOAST: &str = "create-sandbox";

/// One completed CLI job. Carrying the typed result (and any metadata needed
/// to place it) across the channel keeps all state changes on the event-loop
/// thread, where ratcn's state and the modal stack can be updated safely.
#[cfg_attr(test, allow(dead_code))] // production builds construct every variant
pub(super) enum JobResult {
    Sandboxes {
        result: Result<Vec<Sandbox>, String>,
        /// True for a button/shortcut refresh, false for the five-second tick.
        announce: bool,
        /// The first load also needs a secret inventory for the new list.
        then_secrets: bool,
    },
    Secrets {
        result: Result<Vec<Secret>, String>,
        announce: bool,
    },
    Ports {
        name: String,
        /// Guards against a stale load landing in a reopened dialog.
        token: u64,
        result: Result<Vec<PortMapping>, String>,
    },
    PortCommand {
        name: String,
        success: String,
        error_title: &'static str,
        fallback: &'static str,
        result: io::Result<Output>,
    },
    SbxCommand {
        success: String,
        result: io::Result<Output>,
    },
    SecretExists {
        service: String,
        scope: String,
        result: Result<bool, String>,
    },
    SecretSet {
        service: String,
        scope: String,
        success: String,
        result: io::Result<Output>,
    },
    SecretDelete {
        name: String,
        result: io::Result<Output>,
    },
}

/// Sender/receiver pair for the small CLI jobs. Each job gets its own short
/// thread and sends exactly one result; `poll_jobs` drains them without ever
/// waiting on a process.
pub(super) struct Jobs {
    sender: Sender<JobResult>,
    results: Receiver<JobResult>,
}

impl Jobs {
    pub(super) fn new() -> Self {
        let (sender, results) = mpsc::channel();
        Self { sender, results }
    }

    pub(super) fn spawn<F>(&self, job: F)
    where
        F: FnOnce() -> JobResult + Send + 'static,
    {
        let sender = self.sender.clone();
        thread::spawn(move || {
            let _ = sender.send(job());
        });
    }

    pub(super) fn try_recv(&self) -> Option<JobResult> {
        match self.results.try_recv() {
            Ok(result) => Some(result),
            Err(mpsc::TryRecvError::Empty | mpsc::TryRecvError::Disconnected) => None,
        }
    }

    #[cfg(test)]
    pub(super) fn queue_for_test(&self, result: JobResult) {
        self.sender.send(result).expect("job channel");
    }
}

/// Advanced once per event-loop frame while sandbox creation runs in the worker.
pub(super) const SPINNER_FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// A sandbox creation running on a worker thread. The UI keeps rendering (and
/// ignoring input) until the result arrives on the channel.
pub(super) struct Creation {
    pub(super) name: String,
    pub(super) result: Receiver<io::Result<std::process::Output>>,
}

/// One in-flight metrics collection for an open Info dialog. The guest-side
/// CPU sample sleeps for a second, so this runs off the event-loop thread.
pub(super) struct InfoRefresh {
    pub(super) name: String,
    pub(super) result: Receiver<SandboxDetails>,
}

impl App {
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

    /// Advance the spinner once per rendered frame, but only while a sandbox
    /// is actually being created.
    pub fn tick(&mut self) {
        if self.is_creating() {
            self.spinner_index = self.spinner_index.wrapping_add(1);
        }
    }

    /// Pick up a finished sandbox creation without blocking the event loop.
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

    /// Pick up finished CLI jobs. Called once per event-loop frame; every
    /// result is applied synchronously so the UI never reads torn state.
    pub fn poll_jobs(&mut self) {
        while let Some(result) = self.jobs.try_recv() {
            self.apply_job_result(result);
        }
    }

    fn apply_job_result(&mut self, result: JobResult) {
        match result {
            JobResult::Sandboxes {
                result,
                announce,
                then_secrets,
            } => {
                match result {
                    Ok(sandboxes) => {
                        self.apply_statuses(sandboxes);
                        if announce {
                            self.notify_success("Sandbox list refreshed", None);
                        }
                    }
                    Err(error) => self.notify_error("Could not list sandboxes", error),
                }
                if then_secrets {
                    self.refresh_secrets();
                }
            }
            JobResult::Secrets { result, announce } => match result {
                Ok(secrets) => {
                    self.secrets = secrets;
                    if announce {
                        self.notify_success("Secret list refreshed", None);
                    }
                }
                Err(error) => self.notify_error("Could not list secrets", error),
            },
            JobResult::Ports {
                name,
                token,
                result,
            } => {
                // A dialog closed and reopened while the job was running must
                // not receive its table.
                let current = self
                    .ports
                    .as_ref()
                    .is_some_and(|view| view.name == name && view.token == token && view.loading);
                if !current {
                    return;
                }
                let Some(view) = self.ports.as_mut() else {
                    return;
                };
                view.loading = false;
                match result {
                    Ok(ports) => {
                        view.ports = ports;
                        view.error = None;
                    }
                    Err(error) => view.error = Some(error),
                }
            }
            JobResult::PortCommand {
                name,
                success,
                error_title,
                fallback,
                result,
            } => match result {
                Ok(output) if output.status.success() => {
                    let still_open = self.ports.as_ref().is_some_and(|view| view.name == name);
                    if still_open {
                        self.refresh_ports();
                    }
                    self.notify_success(success, None);
                }
                Ok(output) => {
                    let detail = stderr_detail(&output, fallback);
                    self.set_ports_error(&name, detail.clone());
                    self.notify_error(error_title, detail);
                }
                Err(error) => {
                    self.set_ports_error(&name, error.to_string());
                    self.notify_error(error_title, error.to_string());
                }
            },
            JobResult::SbxCommand { success, result } => {
                self.handle_sbx_result(result, success);
            }
            JobResult::SecretExists {
                service,
                scope,
                result,
            } => self.finish_secret_lookup(&service, &scope, result),
            JobResult::SecretSet {
                service,
                scope,
                success,
                result,
            } => match result {
                Ok(output) if output.status.success() => {
                    let still_open = self.secret_form.as_ref().is_some_and(|form| {
                        form.service() == Some(service.as_str())
                            && form.scope() == Some(scope.as_str())
                    });
                    if still_open {
                        self.close_modal(super::SET_SECRET_DIALOG);
                        self.secret_form = None;
                    }
                    self.refresh_secrets();
                    self.notify_success(success, None);
                }
                Ok(output) => self.notify_error(
                    "Could not set secret",
                    stderr_detail(&output, "sbx secret set failed"),
                ),
                Err(error) => self.notify_error("Could not set secret", error.to_string()),
            },
            JobResult::SecretDelete { name, result } => match result {
                Ok(output) if output.status.success() => {
                    self.refresh_secrets();
                    self.notify_success(format!("Deleted secret '{name}'"), None);
                }
                Ok(output) => self.notify_error(
                    "Could not delete secret",
                    stderr_detail(&output, "sbx secrets rm failed"),
                ),
                Err(error) => self.notify_error("Could not delete secret", error.to_string()),
            },
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

    pub(super) fn start_info_refresh(&mut self, sandbox: &Sandbox) {
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

    pub(super) fn start_creation(&mut self, commands: Vec<super::PendingCommand>, name: &str) {
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let mut result = None;
            for pending in commands {
                let output = pending.run();
                let failed = match &output {
                    Ok(output) => !output.status.success(),
                    Err(_) => true,
                };
                result = Some(output);
                if failed {
                    break;
                }
            }
            let _ = sender.send(result.unwrap_or_else(|| Err(io::Error::other("no creation command"))));
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
    }

    pub(super) fn finish_creation(&mut self, result: io::Result<std::process::Output>, name: &str) {
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
