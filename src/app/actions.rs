//! User actions on sandboxes: Start / Stop / Remove, Exec, and the shared
//! handling of their `sbx` command results.

use std::io;

use super::commands::stderr_detail;
use super::{App, CONFIRM_DIALOG, Mode, PendingCommand, Sandbox};

/// An action waiting for the user to confirm it in the modal dialog. Only the
/// name is captured: the state is checked again when the user confirms, so a
/// periodic status refresh cannot start a stopped sandbox by accident.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PendingAction {
    Start(String),
    Stop(String),
    Remove(String),
    /// Secret name followed by its scope (the sandbox name, or "(global)").
    DeleteSecret(String, String),
    /// Service and scope of a secret that already exists. The new value stays
    /// in the secret form until the overwrite is confirmed.
    OverwriteSecret(String, String),
    /// Env key, scope, and placeholder of a custom secret that already exists.
    /// The typed value stays in the custom form until the overwrite is
    /// confirmed; the placeholder lets `sbx secret rm` remove the old entry.
    OverwriteCustomSecret(String, String, String),
}

impl PendingAction {
    fn verb(&self) -> &'static str {
        match self {
            Self::Start(_) => "Start",
            Self::Stop(_) => "Stop",
            Self::Remove(_) => "Remove",
            Self::DeleteSecret(..) => "Delete secret",
            Self::OverwriteSecret(..) | Self::OverwriteCustomSecret(..) => "Overwrite secret",
        }
    }
}

impl App {
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

    /// Start/Stop/Remove and secret deletion never run directly from a button
    /// or shortcut: they first park here so the user can confirm in a dialog.
    pub(super) fn request_action(&mut self, action: PendingAction) {
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
        self.close_modal(CONFIRM_DIALOG);
        self.notify_info(format!("{} cancelled.", action.verb()), None);
    }

    /// The dialog's confirm button. Recheck the sandbox state before acting:
    /// the five-second status ticker may have changed it while the dialog was
    /// open, and a removed sandbox should never be acted on.
    pub fn confirm_action(&mut self) {
        let Some(action) = self.pending_action.take() else {
            return;
        };
        self.close_modal(CONFIRM_DIALOG);
        let should_be_running = matches!(&action, PendingAction::Stop(_));
        let secret_action = matches!(&action, PendingAction::DeleteSecret(..));
        let overwrite_action = matches!(
            &action,
            PendingAction::OverwriteSecret(..) | PendingAction::OverwriteCustomSecret(..)
        );
        let still_valid = match &action {
            PendingAction::Start(name) | PendingAction::Stop(name) => {
                self.has_sandbox(name) && self.is_running(name) == should_be_running
            }
            PendingAction::Remove(name) => self.has_sandbox(name),
            PendingAction::DeleteSecret(name, scope) => self.has_secret(name, scope),
            PendingAction::OverwriteSecret(service, scope) => {
                self.secret_form.as_ref().is_some_and(|form| {
                    form.service() == Some(service.as_str())
                        && form.scope() == Some(scope.as_str())
                })
            }
            PendingAction::OverwriteCustomSecret(env_key, scope, _) => {
                self.custom_secret_form.as_ref().is_some_and(|form| {
                    form.env_key().value().trim() == env_key.as_str()
                        && form.scope() == Some(scope.as_str())
                })
            }
        };
        if still_valid {
            self.perform_action(action);
        } else if secret_action {
            self.notify_info("Secret is no longer listed; action cancelled.", None);
        } else if overwrite_action {
            self.notify_info("Secret form is no longer available; overwrite cancelled.", None);
        } else {
            self.notify_info("Sandbox state changed; action cancelled.", None);
        }
    }

    fn perform_action(&mut self, action: PendingAction) {
        #[cfg(test)]
        {
            if let PendingAction::DeleteSecret(name, scope) = &action {
                self.delete_secret_now(name, scope);
            }
            if let PendingAction::OverwriteSecret(service, scope) = &action {
                self.finish_set_secret(service, scope);
            }
            if let PendingAction::OverwriteCustomSecret(env_key, scope, placeholder) = &action {
                self.deleted_for_test =
                    Some(Self::remove_custom_secret_command(placeholder, scope).preview());
                self.finish_set_custom_secret(env_key, scope);
            }
            self.confirmed_for_test = Some(action);
        }
        #[cfg(not(test))]
        match action {
            PendingAction::Start(name) => self.start_sandbox(&name),
            PendingAction::Stop(name) => self.stop_sandbox(&name),
            PendingAction::Remove(name) => self.remove_sandbox(&name),
            PendingAction::DeleteSecret(name, scope) => self.delete_secret_now(&name, &scope),
            PendingAction::OverwriteSecret(service, scope) => {
                self.finish_set_secret(&service, &scope)
            }
            PendingAction::OverwriteCustomSecret(env_key, scope, placeholder) => {
                self.overwrite_custom_secret(&env_key, &scope, &placeholder)
            }
        }
    }

    #[cfg_attr(test, allow(dead_code))]
    fn start_sandbox(&mut self, name: &str) {
        let command = Self::start_command(name);
        self.run_sbx(command, format!("Sandbox '{name}' started."));
    }

    /// `sbx` has no standalone start verb. `run --name` re-attaches to the
    /// existing sandbox and starts it; `-d` leaves it running detached.
    pub(super) fn start_command(name: &str) -> PendingCommand {
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

    pub(super) fn is_running(&self, name: &str) -> bool {
        self.sandboxes
            .iter()
            .find(|sandbox| sandbox.name == name)
            .is_some_and(Sandbox::is_running)
    }

    pub(super) fn has_sandbox(&self, name: &str) -> bool {
        self.sandboxes.iter().any(|sandbox| sandbox.name == name)
    }

    #[cfg_attr(test, allow(dead_code))]
    fn run_sbx(&mut self, pending: PendingCommand, success: String) {
        self.jobs
            .spawn(move || super::workers::JobResult::SbxCommand {
                success,
                result: pending.run(),
            });
    }

    pub(super) fn handle_sbx_result(
        &mut self,
        result: io::Result<std::process::Output>,
        success: String,
    ) {
        match result {
            Ok(output) if output.status.success() => {
                self.refresh_statuses();
                self.notify_success(success, None);
            }
            Ok(output) => {
                let detail = stderr_detail(&output, "sbx command failed");
                self.notify_error("Could not run sbx", detail);
            }
            Err(error) => self.notify_error("Could not run sbx", error.to_string()),
        }
    }
}
