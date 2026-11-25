//! Test-only constructors and hooks, shared by the app and UI test modules.
//! They keep the production code from growing shell-out branches for tests.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;

use ratcn::Toast;

use super::workers::{CREATE_TOAST, Creation, InfoRefresh};
use super::{
    App, CREATE_DIALOG, INFO_DIALOG, PORTS_DIALOG, PendingAction, PortMapping, PortsView, Sandbox,
    SandboxDetails,
};

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

    /// Point the form at a test directory so the folder picker has real
    /// contents without depending on the developer's home directory.
    pub fn set_workspace_path_for_test(&mut self, path: &Path) {
        self.form.workspace_path = path.to_string_lossy().into_owned();
    }

    /// The command the ports dialog handed to the test hook.
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

static NEXT_WORKSPACE_DIR: AtomicUsize = AtomicUsize::new(0);

/// A real, isolated directory tree for folder-picker tests. Removed on drop so
/// repeated runs do not leave clutter behind.
pub struct TempWorkspaceDir {
    root: PathBuf,
}

impl TempWorkspaceDir {
    pub fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "sbx-tui-workspace-{}-{}",
            std::process::id(),
            NEXT_WORKSPACE_DIR.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&root).expect("create temporary workspace directory");
        Self { root }
    }

    pub fn path(&self) -> &Path {
        &self.root
    }

    pub fn make_dir(&self, name: &str) -> PathBuf {
        let path = self.root.join(name);
        std::fs::create_dir_all(&path).expect("create child directory");
        path
    }
}

impl Drop for TempWorkspaceDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
