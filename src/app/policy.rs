//! First-run network policy setup.
//!
//! `sbx policy ls --json` prints a JSON listing once a global network policy
//! exists. Before that the CLI exits with a plain-text choice prompt, so the
//! TUI asks which policy to use and runs `sbx policy init` for the user.

use std::io;
use std::process::Output;

use serde_json::Value;

use super::{App, PendingCommand};

use self::Setup as PolicySetup;

/// Stable id for the ratcn modal layer.
pub const POLICY_DIALOG: &str = "policy-dialog";

/// The three policies `sbx policy init` accepts, in prompt order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetworkPolicy {
    OpenAll,
    Balanced,
    LockedDown,
}

impl NetworkPolicy {
    pub const ALL: [Self; 3] = [Self::OpenAll, Self::Balanced, Self::LockedDown];

    pub const fn from_index(index: usize) -> Option<Self> {
        match index {
            0 => Some(Self::OpenAll),
            1 => Some(Self::Balanced),
            2 => Some(Self::LockedDown),
            _ => None,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::OpenAll => "Open",
            Self::Balanced => "Balanced",
            Self::LockedDown => "Locked Down",
        }
    }

    pub const fn description(self) -> &'static str {
        match self {
            Self::OpenAll => "all network traffic allowed, no restrictions",
            Self::Balanced => "default deny, with common dev sites allowed",
            Self::LockedDown => "all traffic blocked unless you allow it",
        }
    }

    pub const fn argument(self) -> &'static str {
        match self {
            Self::OpenAll => "allow-all",
            Self::Balanced => "balanced",
            Self::LockedDown => "deny-all",
        }
    }

    pub fn init_command(self) -> PendingCommand {
        PendingCommand::new("sbx", &["policy", "init", self.argument()])
    }
}

/// Result of the startup probe. A valid JSON document means `sbx` is ready;
/// anything else is the uninitialized prompt (or a failure to run the CLI).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StartupCheck {
    Initialized,
    NeedsSetup,
}

/// Run the first CLI command of the session, before the alternate screen opens.
pub fn check_startup() -> StartupCheck {
    let output = PendingCommand::new("sbx", &["policy", "ls", "--json"]).run();
    if listing_is_json(&output) {
        StartupCheck::Initialized
    } else {
        StartupCheck::NeedsSetup
    }
}

/// Only stdout carries the listing. The CLI's uninitialized response is a
/// plain-text prompt on stderr, so it deliberately does not count as JSON.
pub(super) fn listing_is_json(output: &io::Result<Output>) -> bool {
    match output {
        Ok(output) => serde_json::from_slice::<Value>(&output.stdout).is_ok(),
        Err(_) => false,
    }
}

/// State behind the setup dialog: which row holds the cursor, the committed
/// choice, and whether its CLI job is still running.
pub(super) struct Setup {
    pub focus: usize,
    pub selection: Option<usize>,
    pub pending: bool,
}

impl Setup {
    pub fn new() -> Self {
        Self {
            focus: 0,
            selection: None,
            pending: false,
        }
    }
}

impl App {
    /// The setup dialog is deliberately not dismissable: `sbx` cannot manage
    /// sandboxes until a policy exists.
    pub fn require_policy_setup(&mut self) {
        self.policy_setup = Some(PolicySetup::new());
        let _ = self.modals.open(POLICY_DIALOG, &mut self.focus);
    }

    pub fn needs_policy_setup(&self) -> bool {
        self.policy_setup.is_some()
    }

    pub fn policy_focus(&self) -> usize {
        self.policy_setup.as_ref().map_or(0, |setup| setup.focus)
    }

    pub fn policy_selection(&self) -> Option<usize> {
        self.policy_setup.as_ref().and_then(|setup| setup.selection)
    }

    pub fn policy_pending(&self) -> bool {
        self.policy_setup
            .as_ref()
            .is_some_and(|setup| setup.pending)
    }

    pub fn set_policy_focus(&mut self, index: usize) {
        if let Some(setup) = self.policy_setup.as_mut()
            && NetworkPolicy::from_index(index).is_some()
        {
            setup.focus = index;
        }
    }

    /// Commit a choice and run `sbx policy init` on a worker. The dialog stays
    /// open until the CLI reports success, so a failure can be retried.
    pub fn select_network_policy(&mut self, index: usize) {
        let Some(policy) = NetworkPolicy::from_index(index) else {
            return;
        };
        if self
            .policy_setup
            .as_ref()
            .is_none_or(|setup| setup.pending)
        {
            return;
        }
        if let Some(setup) = self.policy_setup.as_mut() {
            setup.focus = index;
            setup.selection = Some(index);
            setup.pending = true;
        }
        let command = policy.init_command();
        self.notify_info(
            format!("Initializing {} network policy...", policy.label()),
            None,
        );

        #[cfg(test)]
        {
            self.policy_command_for_test = Some(command.preview());
        }
        #[cfg(not(test))]
        self.jobs
            .spawn(move || super::workers::JobResult::PolicyInit {
                result: command.run(),
            });
    }

    /// Apply a finished `sbx policy init` result. A success closes the dialog
    /// and starts the dashboard loads that were held back at startup.
    pub(super) fn finish_policy_init(&mut self, result: io::Result<Output>) {
        let policy = match self.policy_setup.as_mut() {
            Some(setup) => {
                setup.pending = false;
                setup.selection.and_then(NetworkPolicy::from_index)
            }
            None => return,
        };
        match result {
            Ok(output) if output.status.success() => {
                let label = policy.map_or("Network", NetworkPolicy::label);
                self.policy_setup = None;
                self.close_modal(POLICY_DIALOG);
                self.notify_success(format!("{label} network policy initialized."), None);
                #[cfg(not(test))]
                self.start();
            }
            Ok(output) => {
                let detail = super::commands::stderr_detail(&output, "sbx policy init failed");
                self.notify_error("Could not initialize network policy", detail);
                if let Some(setup) = self.policy_setup.as_mut() {
                    setup.selection = None;
                }
            }
            Err(error) => {
                self.notify_error("Could not initialize network policy", error.to_string());
                if let Some(setup) = self.policy_setup.as_mut() {
                    setup.selection = None;
                }
            }
        }
    }
}
