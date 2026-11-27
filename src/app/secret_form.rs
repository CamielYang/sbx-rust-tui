//! State for the Set secret and Set custom secret dialogs.
//!
//! The service and scope choices are index-backed so ratcn's `Select` can read
//! and replace them as plain values. The scope list is snapshotted when the
//! dialog opens: a periodic sandbox refresh must not shift the option under the
//! user's cursor. The custom form has no choices — just an env key, a secret
//! value, and a growable list of hosts.

use ratcn::InputState;

use super::Sandbox;

/// Services `sbx secret set` recognises, in the order offered in the dialog.
pub const SERVICES: &[&str] = &[
    "anthropic",
    "cursor",
    "droid",
    "github",
    "google",
    "groq",
    "mistral",
    "nebius",
    "openai",
    "openrouter",
    "xai",
];

/// The first scope option. It stands for the global secret store, which omits
/// `--sandbox` from the CLI command.
pub const GLOBAL_SCOPE: &str = "(global)";

/// Only the tagged `(global)` scope omits `--sandbox`. A sandbox may itself be
/// named `global`, so that spelling must still be treated as a sandbox.
pub fn is_global_scope(scope: &str) -> bool {
    scope == GLOBAL_SCOPE
}

#[derive(Clone)]
pub struct SecretForm {
    pub(super) service_index: usize,
    pub(super) service_focus: usize,
    pub(super) service_open: bool,
    pub(super) scopes: Vec<String>,
    pub(super) scope_index: usize,
    pub(super) scope_focus: usize,
    pub(super) scope_open: bool,
    pub(super) secret: InputState,
    /// Set once **Set secret** is pressed, so an empty value can be called out
    /// inside the dialog as well as in a toast.
    pub(super) attempted: bool,
}

impl SecretForm {
    pub(super) fn new(sandboxes: &[Sandbox]) -> Self {
        let mut scopes = vec![GLOBAL_SCOPE.to_owned()];
        scopes.extend(sandboxes.iter().map(|sandbox| sandbox.name.clone()));
        Self {
            service_index: 0,
            service_focus: 0,
            service_open: false,
            scopes,
            scope_index: 0,
            scope_focus: 0,
            scope_open: false,
            secret: InputState::new(""),
            attempted: false,
        }
    }

    pub fn service_index(&self) -> usize {
        self.service_index
    }

    pub fn service_focus(&self) -> usize {
        self.service_focus
    }

    pub fn service_open(&self) -> bool {
        self.service_open
    }

    pub fn scopes(&self) -> &[String] {
        &self.scopes
    }

    pub fn scope_index(&self) -> usize {
        self.scope_index
    }

    pub fn scope_focus(&self) -> usize {
        self.scope_focus
    }

    pub fn scope_open(&self) -> bool {
        self.scope_open
    }

    pub fn secret(&self) -> &InputState {
        &self.secret
    }

    pub fn attempted(&self) -> bool {
        self.attempted
    }

    /// The committed service name, or `None` if the index went stale.
    pub fn service(&self) -> Option<&'static str> {
        SERVICES.get(self.service_index).copied()
    }

    /// The committed scope: `"(global)"` or a sandbox name.
    pub fn scope(&self) -> Option<&str> {
        self.scopes.get(self.scope_index).map(String::as_str)
    }

    pub fn set_service_index(&mut self, index: usize) {
        self.service_index = index.min(SERVICES.len().saturating_sub(1));
        self.service_focus = self.service_index;
        // Committing a choice also closes the panel, as in the create wizard.
        self.service_open = false;
    }

    pub fn set_service_focus(&mut self, index: usize) {
        self.service_focus = index.min(SERVICES.len().saturating_sub(1));
    }

    pub fn set_service_open(&mut self, open: bool) {
        self.service_open = open;
    }

    pub fn set_scope_index(&mut self, index: usize) {
        self.scope_index = index.min(self.scopes.len().saturating_sub(1));
        self.scope_focus = self.scope_index;
        self.scope_open = false;
    }

    pub fn set_scope_focus(&mut self, index: usize) {
        self.scope_focus = index.min(self.scopes.len().saturating_sub(1));
    }

    pub fn set_scope_open(&mut self, open: bool) {
        self.scope_open = open;
    }

    pub fn set_secret(&mut self, state: InputState) {
        self.secret = state;
    }

    pub fn mark_attempted(&mut self) {
        self.attempted = true;
    }
}

/// State for the Set custom secret dialog. Each host is its own input so the
/// dialog can send one `--host` per filled row, in the order shown.
#[derive(Clone)]
pub struct CustomSecretForm {
    pub(super) env_key: InputState,
    pub(super) secret: InputState,
    pub(super) hosts: Vec<InputState>,
    /// Set once **Set custom secret** is pressed, so empty fields can be
    /// called out inside the dialog as well as in a toast.
    pub(super) attempted: bool,
}

impl CustomSecretForm {
    pub(super) fn new() -> Self {
        Self {
            env_key: InputState::new(""),
            secret: InputState::new(""),
            hosts: vec![InputState::new("")],
            attempted: false,
        }
    }

    pub fn env_key(&self) -> &InputState {
        &self.env_key
    }

    pub fn secret(&self) -> &InputState {
        &self.secret
    }

    pub fn hosts(&self) -> &[InputState] {
        &self.hosts
    }

    pub fn attempted(&self) -> bool {
        self.attempted
    }

    pub fn set_env_key(&mut self, state: InputState) {
        self.env_key = state;
    }

    pub fn set_secret(&mut self, state: InputState) {
        self.secret = state;
    }

    pub fn set_host(&mut self, index: usize, state: InputState) {
        if let Some(host) = self.hosts.get_mut(index) {
            *host = state;
        }
    }

    pub fn add_host(&mut self) {
        self.hosts.push(InputState::new(""));
    }

    pub fn mark_attempted(&mut self) {
        self.attempted = true;
    }
}
