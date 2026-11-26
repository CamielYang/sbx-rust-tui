//! Modal dialog lifecycle: the create wizard and the sandbox/secret Info dialog.

use super::{App, CREATE_DIALOG, Form, INFO_DIALOG, SandboxDetails};

impl App {
    /// Close a modal only when it is the topmost layer. Each dialog also clears
    /// its own data so a stale result cannot leak into the next one.
    pub(super) fn close_modal(&mut self, id: &str) {
        if self.modals.top().is_some_and(|top| top.as_str() == id) {
            self.modals.close(&mut self.focus);
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

    pub(super) fn close_wizard(&mut self) {
        self.close_modal(CREATE_DIALOG);
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
        let commands = vec![self.form.create_command(), self.form.keep_alive_command()];
        let name = self.form.name.value().trim().to_owned();
        self.start_creation(commands, &name);
    }

    pub fn open_info(&mut self, name: &str) {
        let Some(sandbox) = self.sandboxes.iter().find(|s| s.name == name).cloned() else {
            return;
        };
        self.select_named(name);
        // Show the known values immediately; the guest-side metrics arrive on
        // a worker so the one-second CPU sample never freezes the TUI.
        self.secret_info = None;
        self.info = Some(SandboxDetails {
            name: sandbox.name.clone(),
            status: sandbox.status.clone(),
            ..SandboxDetails::default()
        });
        self.info_refresh = None;
        self.start_info_refresh(&sandbox);
        let _ = self.modals.open(INFO_DIALOG, &mut self.focus);
    }

    /// A clicked secret row opens the same Info dialog with its own details.
    /// The lookup uses the current table so a stale row cannot show old data.
    pub fn open_secret_info(&mut self, name: &str, scope: &str) {
        let Some(secret) = self
            .secrets
            .iter()
            .find(|secret| secret.name == name && secret.scope == scope)
            .cloned()
        else {
            return;
        };
        // Secret details are static: no sandbox metrics worker should touch
        // this dialog or replace its content.
        self.info = None;
        self.info_refresh = None;
        self.secret_info = Some(secret);
        let _ = self.modals.open(INFO_DIALOG, &mut self.focus);
    }

    pub fn close_info(&mut self) {
        self.close_modal(INFO_DIALOG);
        // Invalidate any worker still running: its result must not land in a
        // dialog opened later for a different sandbox.
        self.info = None;
        self.secret_info = None;
        self.info_refresh = None;
    }

    pub fn open_selected_info(&mut self) {
        if let Some(name) = self.selected_name()
            && self.is_running(&name)
        {
            self.open_info(&name);
        }
    }
}
