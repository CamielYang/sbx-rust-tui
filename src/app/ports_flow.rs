//! The published-ports dialog: loading the table, publishing and unpublishing.

#[cfg(not(test))]
use super::ports::PortField;
use super::ports::{PortForm, PortsView, fetch_ports};
use super::{App, Mode, PORTS_DIALOG, PendingCommand};

impl App {
    /// Show the published ports of the sandbox the Actions pane targets.
    pub fn open_ports_selected(&mut self) {
        if let Some(name) = self.selected_name()
            && self.is_running(&name)
        {
            self.open_ports(&name);
        }
    }

    /// Open the ports dialog immediately and start the listing on a worker.
    /// The dialog shows a loading hint until the result lands, so opening it
    /// never waits on `sbx port ls`.
    pub(super) fn open_ports(&mut self, name: &str) {
        if !self.is_running(name) {
            return;
        }
        self.select_named(name);
        self.port_form = PortForm::new();
        self.ports_token = self.ports_token.wrapping_add(1);
        self.ports = Some(PortsView {
            name: name.to_owned(),
            ports: Vec::new(),
            error: None,
            loading: true,
            token: self.ports_token,
        });
        let _ = self.modals.open(PORTS_DIALOG, &mut self.focus);
        self.refresh_ports();
    }

    pub fn close_ports(&mut self) {
        self.close_modal(PORTS_DIALOG);
        self.ports = None;
    }

    /// Re-read the table on a worker, keeping the dialog interactive. A token
    /// ties the result to this specific load attempt.
    #[cfg_attr(test, allow(dead_code))]
    pub(super) fn refresh_ports(&mut self) {
        let Some(view) = self.ports.as_mut() else {
            return;
        };
        let name = view.name.clone();
        self.ports_token = self.ports_token.wrapping_add(1);
        view.token = self.ports_token;
        view.loading = true;
        let token = self.ports_token;
        let fetch_name = name.clone();
        self.jobs.spawn(move || {
            let result = fetch_ports(&fetch_name);
            super::workers::JobResult::Ports {
                name,
                token,
                result,
            }
        });
    }

    /// Surface a failed publish/unpublish in the dialog that requested it,
    /// without touching a dialog opened for a different sandbox.
    pub(super) fn set_ports_error(&mut self, name: &str, error: String) {
        if let Some(view) = self.ports.as_mut()
            && view.name == name
        {
            view.error = Some(error);
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
        {
            let host = self.port_form.port(PortField::Host).unwrap_or(0);
            let sandbox = self.port_form.port(PortField::Sandbox).unwrap_or(0);
            let success = format!("Published {host}:{sandbox} for '{name}'.");
            self.notify_info(format!("Publishing {host}:{sandbox} for '{name}'..."), None);
            self.jobs
                .spawn(move || super::workers::JobResult::PortCommand {
                    name,
                    success,
                    error_title: "Could not publish port",
                    fallback: "sbx port publish failed",
                    result: pending.run(),
                });
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
        {
            let success = format!("Unpublished {host}:{sandbox} for '{name}'.");
            self.notify_info(
                format!("Unpublishing {host}:{sandbox} for '{name}'..."),
                None,
            );
            self.jobs
                .spawn(move || super::workers::JobResult::PortCommand {
                    name,
                    success,
                    error_title: "Could not unpublish port",
                    fallback: "sbx port unpublish failed",
                    result: pending.run(),
                });
        }
    }
}
