use std::process::Command as ProcessCommand;

use ratcn::InputState;
use serde_json::Value;

use super::{PendingCommand, sandbox::json_text};

/// One row of the published-ports table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PortMapping {
    pub host_port: u16,
    pub sandbox_port: u16,
    pub protocol: String,
    pub host_ip: Option<String>,
}

/// The ports currently published for the sandbox named in the dialog.
#[derive(Clone, Debug)]
pub struct PortsView {
    pub name: String,
    pub ports: Vec<PortMapping>,
    pub error: Option<String>,
    /// True while a listing job is in flight, so the dialog can say so.
    pub loading: bool,
    /// Identifies the current listing; late results with an older token are dropped.
    pub token: u64,
}

/// Which end of a `host:sandbox` mapping is being edited.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PortField {
    Host,
    Sandbox,
}

impl PortField {
    pub fn label(self) -> &'static str {
        match self {
            Self::Host => "Host port",
            Self::Sandbox => "Sandbox port",
        }
    }
}

/// The two editable ports behind the modal's **Add port** button. The defaults
/// match `sbx port publish <name> 8080:3000` so the button is useful on its own.
#[derive(Clone)]
pub struct PortForm {
    pub(super) host: InputState,
    pub(super) sandbox: InputState,
}

impl PortForm {
    pub(super) fn new() -> Self {
        Self {
            host: InputState::new("8080"),
            sandbox: InputState::new("3000"),
        }
    }

    pub fn host(&self) -> &InputState {
        &self.host
    }

    pub fn sandbox(&self) -> &InputState {
        &self.sandbox
    }

    pub(super) fn port(&self, field: PortField) -> Result<u16, String> {
        let state = match field {
            PortField::Host => &self.host,
            PortField::Sandbox => &self.sandbox,
        };
        parse_port(state.value())
    }

    /// True when an input cannot be turned into a usable port.
    pub fn invalid(&self, field: PortField) -> bool {
        self.port(field).is_err()
    }

    pub fn first_error(&self) -> Option<String> {
        [PortField::Host, PortField::Sandbox]
            .into_iter()
            .find_map(|field| {
                self.port(field)
                    .err()
                    .map(|error| format!("{}: {error}", field.label()))
            })
    }

    pub(super) fn pending_command(&self, name: &str) -> PendingCommand {
        // Callers validate first; the fallbacks keep this helper total.
        let host = self.port(PortField::Host).unwrap_or(0);
        let sandbox = self.port(PortField::Sandbox).unwrap_or(0);
        PendingCommand::new(
            "sbx",
            &["port", "publish", name, &format!("{host}:{sandbox}")],
        )
    }
}

fn parse_port(value: &str) -> Result<u16, String> {
    let value = value.trim();
    match value.parse::<u16>() {
        Ok(port) if port > 0 => Ok(port),
        _ => Err("Enter a port between 1 and 65535.".to_owned()),
    }
}

/// Read published ports with `sbx port ls <name> --format json`. The input is
/// an array (or an object with a `ports` array), as emitted by current `sbx`.
pub fn fetch_ports(name: &str) -> Result<Vec<PortMapping>, String> {
    let output = ProcessCommand::new("sbx")
        .args(["port", "ls", name, "--format", "json"])
        .output()
        .map_err(|error| format!("Could not run sbx port ls: {error}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(if stderr.is_empty() {
            format!("sbx port ls exited with {}.", output.status)
        } else {
            stderr
        });
    }

    parse_ports(&output.stdout)
}

pub(super) fn parse_ports(stdout: &[u8]) -> Result<Vec<PortMapping>, String> {
    let value: Value = serde_json::from_slice(stdout)
        .map_err(|error| format!("Could not parse sbx port ls output: {error}"))?;
    let rows = value
        .as_array()
        .or_else(|| value.get("ports").and_then(Value::as_array))
        .ok_or_else(|| "Unexpected sbx port ls output.".to_owned())?;

    Ok(rows
        .iter()
        .filter_map(|row| {
            let host = json_text(row, &["host_port", "hostPort", "host"])?;
            let sandbox = json_text(
                row,
                &[
                    "sandbox_port",
                    "sandboxPort",
                    "guest_port",
                    "container_port",
                    "port",
                ],
            )?;
            Some(PortMapping {
                host_port: parse_port(&host).ok()?,
                sandbox_port: parse_port(&sandbox).ok()?,
                protocol: json_text(row, &["protocol"]).unwrap_or_else(|| "tcp".to_owned()),
                host_ip: json_text(row, &["host_ip", "hostIp"]),
            })
        })
        .collect())
}
