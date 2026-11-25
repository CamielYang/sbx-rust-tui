//! The published-ports dialog: loading the table, publishing and unpublishing.

#[cfg(not(test))]
use super::commands::stderr_detail;
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

    /// Open (or refill) the ports dialog. Listing ports is a quick local `sbx`
    /// call, so the table is ready on the first frame; a failure is shown in
    /// the dialog instead of blocking it.
    pub(super) fn open_ports(&mut self, name: &str) {
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
        self.close_modal(PORTS_DIALOG);
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
        {
            let host = self.port_form.port(PortField::Host).unwrap_or(0);
            let sandbox = self.port_form.port(PortField::Sandbox).unwrap_or(0);
            let success = format!("Published {host}:{sandbox} for '{name}'.");
            self.run_port_command(
                pending,
                success,
                "Could not publish port",
                "sbx port publish failed",
            );
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
            self.run_port_command(
                pending,
                success,
                "Could not unpublish port",
                "sbx port unpublish failed",
            );
        }
    }

    /// Both port commands report failures the same way: in the dialog's error
    /// line and in a toast. Success refreshes the table and shows a toast.
    #[cfg(not(test))]
    fn run_port_command(
        &mut self,
        pending: PendingCommand,
        success: String,
        error_title: &'static str,
        fallback: &'static str,
    ) {
        let detail = match pending.run() {
            Ok(output) if output.status.success() => {
                self.refresh_ports();
                self.notify_success(success, None);
                return;
            }
            Ok(output) => stderr_detail(&output, fallback),
            Err(error) => error.to_string(),
        };
        if let Some(view) = self.ports.as_mut() {
            view.error = Some(detail.clone());
        }
        self.notify_error(error_title, detail);
    }
}
