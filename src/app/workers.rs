//! Background work: `sbx create` and the Info dialog's guest-side metrics.
//! Both run on worker threads so the event loop keeps rendering.

use std::io;
use std::sync::mpsc::{self, Receiver};
use std::thread;

use ratcn::Toast;

use super::{App, Sandbox, SandboxDetails, inspect_sandbox, sandbox::push_error};

pub(super) const CREATE_TOAST: &str = "create-sandbox";

/// Advanced once per event-loop frame while `sbx create` runs in the worker.
pub(super) const SPINNER_FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// A `sbx create` running on a worker thread. The UI keeps rendering (and
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

    pub(super) fn start_creation(&mut self, pending: super::PendingCommand, name: &str) {
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
