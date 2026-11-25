//! The workspace folder picker opened by the create dialog's **Change** button.

use std::fs;
use std::path::{Path, PathBuf};

use super::{App, CREATE_DIALOG, WORKSPACE_DIALOG};

/// The first row of any directory that has a parent. Selecting it goes up one
/// level instead of into a child folder.
pub const WORKSPACE_PARENT: &str = "..";

/// One directory's contents, kept between frames so browsing does not re-read
/// the filesystem on every render.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkspacePicker {
    dir: PathBuf,
    entries: Vec<String>,
    focused: Option<String>,
    scroll: usize,
    error: Option<String>,
}

impl WorkspacePicker {
    /// Start at the configured directory. If it no longer exists, fall back to
    /// the nearest ancestor that does so the browser always has a real place
    /// to list.
    pub(super) fn open(path: &str) -> Self {
        let requested = PathBuf::from(path);
        let dir = nearest_dir(&requested);
        let mut picker = Self {
            dir,
            entries: Vec::new(),
            focused: None,
            scroll: 0,
            error: None,
        };
        picker.reload();
        picker
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn entries(&self) -> &[String] {
        &self.entries
    }

    pub fn focused(&self) -> Option<&str> {
        self.focused.as_deref()
    }

    pub fn scroll(&self) -> usize {
        self.scroll
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub(super) fn set_focus(&mut self, name: String, scroll: usize) {
        self.focused = Some(name);
        self.scroll = scroll;
    }

    pub(super) fn set_scroll(&mut self, offset: usize) {
        self.scroll = offset;
    }

    /// Enter a child folder or, on `..`, go to the parent. Unknown rows are
    /// ignored so a late click against a refreshed directory cannot navigate
    /// somewhere the list no longer shows.
    pub(super) fn select(&mut self, name: &str) {
        if name == WORKSPACE_PARENT {
            if let Some(parent) = self.dir.parent() {
                self.dir = parent.to_path_buf();
                self.reload();
            }
            return;
        }
        if self.entries.iter().any(|entry| entry == name) {
            self.dir.push(name);
            self.reload();
        }
    }

    /// Re-list the current directory and put the cursor on its first row.
    fn reload(&mut self) {
        match fs::read_dir(&self.dir) {
            Ok(entries) => {
                let mut names = Vec::new();
                for entry in entries.flatten() {
                    if entry.path().is_dir()
                        && let Some(name) = entry.file_name().to_str()
                    {
                        names.push(name.to_owned());
                    }
                }
                names.sort();
                if self.dir.parent().is_some() {
                    names.insert(0, WORKSPACE_PARENT.to_owned());
                }
                self.entries = names;
                self.error = None;
            }
            Err(error) => {
                self.entries = if self.dir.parent().is_some() {
                    vec![WORKSPACE_PARENT.to_owned()]
                } else {
                    Vec::new()
                };
                self.error = Some(format!("Could not read this folder: {error}"));
            }
        }
        self.focused = self.entries.first().cloned();
        self.scroll = 0;
    }
}

fn nearest_dir(path: &Path) -> PathBuf {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().unwrap_or_default().join(path)
    };
    let mut candidate = absolute.as_path();
    loop {
        if candidate.is_dir() {
            return candidate.to_path_buf();
        }
        let Some(parent) = candidate.parent() else {
            break;
        };
        if parent == candidate {
            break;
        }
        candidate = parent;
    }
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"))
}

impl App {
    /// Open the browser on top of the create wizard, starting where the form's
    /// workspace path currently points.
    pub fn open_workspace_picker(&mut self) {
        if self
            .modals
            .top()
            .is_none_or(|top| top.as_str() != CREATE_DIALOG)
        {
            return;
        }
        self.workspace_picker = Some(WorkspacePicker::open(self.form.workspace_path()));
        let _ = self.modals.open(WORKSPACE_DIALOG, &mut self.focus);
    }

    pub fn cancel_workspace_picker(&mut self) {
        self.close_modal(WORKSPACE_DIALOG);
        self.workspace_picker = None;
    }

    /// **Create here** commits the folder being browsed and returns to the
    /// wizard; cancelling leaves the original path untouched.
    pub fn choose_workspace_dir(&mut self) {
        if self
            .modals
            .top()
            .is_none_or(|top| top.as_str() != WORKSPACE_DIALOG)
        {
            return;
        }
        self.close_modal(WORKSPACE_DIALOG);
        if let Some(picker) = self.workspace_picker.take() {
            self.form.workspace_path = picker.dir.to_string_lossy().into_owned();
        }
    }

    pub fn focus_workspace_entry(&mut self, name: String, scroll: usize) {
        if let Some(picker) = self.workspace_picker.as_mut() {
            picker.set_focus(name, scroll);
        }
    }

    pub fn set_workspace_scroll(&mut self, offset: usize) {
        if let Some(picker) = self.workspace_picker.as_mut() {
            picker.set_scroll(offset);
        }
    }

    pub fn select_workspace_entry(&mut self, name: String) {
        if let Some(picker) = self.workspace_picker.as_mut() {
            picker.select(&name);
        }
    }
}
