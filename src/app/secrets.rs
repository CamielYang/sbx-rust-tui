//! Reading the secret inventory shown in the Secrets tab.

use std::process::Command as ProcessCommand;

use serde_json::Value;

#[cfg(not(test))]
use super::commands::stderr_detail;
use super::sandbox::json_text;
use super::secret_form::{GLOBAL_SCOPE, SecretForm};
use super::{
    App, CONFIRM_DIALOG, Mode, PendingAction, PendingCommand, SET_SECRET_DIALOG, service_info,
};

/// One row of `sbx secret ls --format json`, reduced to the fields the
/// dashboard table shows. Service metadata is resolved from the name so a row
/// carries everything its info dialog needs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Secret {
    pub scope: String,
    pub name: String,
    pub env_vars: Vec<String>,
    pub api_domains: Vec<String>,
}

impl Secret {
    pub fn new(scope: impl Into<String>, name: impl Into<String>) -> Self {
        let name = name.into();
        let info = service_info(&name);
        Self {
            scope: scope.into(),
            // Custom secrets are keyed by their env name; there is no service
            // entry to look up, so the name itself is the variable they use.
            name: name.clone(),
            env_vars: info.map_or_else(
                || vec![name.clone()],
                |info| info.env_vars.iter().map(|env| (*env).to_owned()).collect(),
            ),
            api_domains: info.map_or_else(Vec::new, |info| {
                info.api_domains
                    .iter()
                    .map(|domain| (*domain).to_owned())
                    .collect()
            }),
        }
    }

    /// The ENV VAR cell and the info dialog's value for one row.
    pub fn env_vars_label(&self) -> String {
        self.env_vars.join(", ")
    }
}

/// List stored secrets. The CLI reports the actual values as `(stored)`, so
/// nothing sensitive is kept in the app state.
pub fn fetch_secrets() -> Result<Vec<Secret>, String> {
    let output = ProcessCommand::new("sbx")
        .args(["secret", "ls", "--format", "json"])
        .output()
        .map_err(|error| error.to_string())?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(if stderr.is_empty() {
            format!("sbx secret ls exited with {}.", output.status)
        } else {
            stderr
        });
    }

    parse_secrets(&output.stdout)
}

/// Both `secrets` and `custom_secrets` are listed; the latter has no service
/// scope of its own, so it is grouped under "custom".
fn parse_secrets(bytes: &[u8]) -> Result<Vec<Secret>, String> {
    let value: Value = serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
    let mut secrets = Vec::new();

    for (field, fallback_scope) in [("secrets", "global"), ("custom_secrets", "custom")] {
        let Some(rows) = value.get(field).and_then(Value::as_array) else {
            continue;
        };
        for row in rows {
            let Some(name) = json_text(row, &["name", "service", "env_name"]) else {
                continue;
            };
            let scope = json_text(row, &["scope", "sandbox"])
                .unwrap_or_else(|| fallback_scope.to_owned());
            secrets.push(Secret::new(scope, name));
        }
    }

    if value.get("secrets").and_then(Value::as_array).is_none() {
        return Err("sbx secret ls did not include a secret list".to_owned());
    }
    Ok(secrets)
}

/// Whether a scoped `sbx secret ls --format json` reports a stored secret.
/// Custom secrets are keyed by env name rather than service, so only the
/// `secrets` array answers the overwrite question.
fn parse_secret_exists(bytes: &[u8]) -> Result<bool, String> {
    let value: Value = serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
    let secrets = value
        .get("secrets")
        .and_then(Value::as_array)
        .ok_or_else(|| "sbx secret ls did not include a secret list".to_owned())?;
    Ok(!secrets.is_empty())
}

impl App {
    /// A queued deletion is skipped if the row is no longer in the table when
    /// the dialog is confirmed (e.g. after a manual Refresh).
    pub(super) fn has_secret(&self, name: &str, scope: &str) -> bool {
        self.secrets
            .iter()
            .any(|secret| secret.name == name && secret.scope == scope)
    }

    /// Open the Set secret dialog. Scope options are snapshot from the current
    /// sandbox list, with `"(global)"` first.
    pub fn set_secret(&mut self) {
        if self.mode() != Mode::Normal {
            return;
        }
        self.secret_form = Some(SecretForm::new(&self.sandboxes));
        let _ = self.modals.open(SET_SECRET_DIALOG, &mut self.focus);
    }

    pub fn cancel_set_secret(&mut self) {
        self.close_modal(SET_SECRET_DIALOG);
        self.secret_form = None;
        self.notify_info("Set secret cancelled.", None);
    }

    pub fn set_secret_service_open(&mut self, open: bool) {
        if let Some(form) = self.secret_form.as_mut() {
            form.set_service_open(open);
        }
    }

    pub fn set_secret_service_focus(&mut self, index: usize) {
        if let Some(form) = self.secret_form.as_mut() {
            form.set_service_focus(index);
        }
    }

    pub fn set_secret_service(&mut self, index: usize) {
        if let Some(form) = self.secret_form.as_mut() {
            form.set_service_index(index);
        }
    }

    pub fn set_secret_scope_open(&mut self, open: bool) {
        if let Some(form) = self.secret_form.as_mut() {
            form.set_scope_open(open);
        }
    }

    pub fn set_secret_scope_focus(&mut self, index: usize) {
        if let Some(form) = self.secret_form.as_mut() {
            form.set_scope_focus(index);
        }
    }

    pub fn set_secret_scope(&mut self, index: usize) {
        if let Some(form) = self.secret_form.as_mut() {
            form.set_scope_index(index);
        }
    }

    pub fn set_secret_state(&mut self, state: ratcn::InputState) {
        if let Some(form) = self.secret_form.as_mut() {
            form.set_secret(state);
        }
    }

    /// The dialog's **Set secret** button. Look for an existing secret first:
    /// overwriting one goes through the shared confirmation dialog, stacked
    /// above the form so a cancel returns with the typed value intact.
    pub fn submit_secret(&mut self) {
        if self.mode() != Mode::SetSecret {
            return;
        }
        let Some(form) = self.secret_form.as_mut() else {
            return;
        };
        form.mark_attempted();
        let service = form.service().map(str::to_owned);
        let scope = form.scope().map(str::to_owned);
        let secret = form.secret().value().trim().to_owned();

        let (Some(service), Some(scope)) = (service, scope) else {
            self.notify_error("Invalid secret", "Choose a service and a scope.");
            return;
        };
        if secret.is_empty() {
            self.notify_error("Invalid secret", "Enter a secret value.");
            return;
        }

        let exists = match self.stored_secret_exists(&service, &scope) {
            Ok(exists) => exists,
            Err(error) => {
                self.notify_error("Could not check secret", error);
                return;
            }
        };
        if exists {
            self.pending_action = Some(PendingAction::OverwriteSecret(service, scope));
            let _ = self.modals.open(CONFIRM_DIALOG, &mut self.focus);
            return;
        }
        self.finish_set_secret(&service, &scope);
    }

    /// `sbx secret ls --sandbox <scope> --service <service> --format json`.
    /// The global store has no sandbox, so it uses `--global` instead.
    pub(super) fn secret_lookup_command(service: &str, scope: &str) -> PendingCommand {
        let mut args = vec!["secret", "ls"];
        if scope == GLOBAL_SCOPE {
            args.push("--global");
        } else {
            args.extend(["--sandbox", scope]);
        }
        args.extend(["--service", service, "--format", "json"]);
        PendingCommand::new("sbx", &args)
    }

    /// Ask `sbx` whether this service already has a secret in the scope. Tests
    /// supply the answer instead of shelling out.
    #[cfg(test)]
    fn stored_secret_exists(&self, _service: &str, _scope: &str) -> Result<bool, String> {
        Ok(self.secret_exists_for_test.unwrap_or(false))
    }

    #[cfg(not(test))]
    fn stored_secret_exists(&self, service: &str, scope: &str) -> Result<bool, String> {
        let output = Self::secret_lookup_command(service, scope)
            .run()
            .map_err(|error| error.to_string())?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
            return Err(if stderr.is_empty() {
                format!("sbx secret ls exited with {}.", output.status)
            } else {
                stderr
            });
        }
        parse_secret_exists(&output.stdout)
    }

    /// Run the set command for the value still held by the form, then close the
    /// dialog and refresh the table. Shared by the direct and confirmed paths.
    pub(super) fn finish_set_secret(&mut self, service: &str, scope: &str) {
        let Some(secret) = self
            .secret_form
            .as_ref()
            .map(|form| form.secret().value().trim().to_owned())
        else {
            return;
        };
        let pending = Self::set_secret_command(service, scope, &secret);
        let target = if scope == GLOBAL_SCOPE {
            "global".to_owned()
        } else {
            scope.to_owned()
        };
        let success = format!("Set secret '{service}' for {target}.");

        #[cfg(test)]
        {
            self.set_for_test = Some(pending.preview());
            self.close_modal(SET_SECRET_DIALOG);
            self.secret_form = None;
            self.notify_success(success, None);
        }

        #[cfg(not(test))]
        {
            let detail = match pending.run() {
                Ok(output) if output.status.success() => {
                    self.close_modal(SET_SECRET_DIALOG);
                    self.secret_form = None;
                    self.refresh_secrets();
                    self.notify_success(success, None);
                    return;
                }
                Ok(output) => stderr_detail(&output, "sbx secret set failed"),
                Err(error) => error.to_string(),
            };
            self.notify_error("Could not set secret", detail);
        }
    }

    /// `echo "<value>" | sbx secret set <service> [--sandbox <scope>] -f`
    /// Global secrets omit the sandbox flag entirely.
    fn set_secret_command(service: &str, scope: &str, secret: &str) -> PendingCommand {
        let mut command = format!(
            "echo \"{}\" | sbx secret set {}",
            shell_double_quoted(secret),
            service
        );
        if scope != GLOBAL_SCOPE {
            command.push_str(" --sandbox ");
            command.push_str(&shell_word(scope));
        }
        command.push_str(" -f");
        PendingCommand::shell(command)
    }
}

/// Escape the characters that would end or expand a double-quoted shell word.
fn shell_double_quoted(value: &str) -> String {
    let mut escaped = String::new();
    for character in value.chars() {
        if matches!(character, '\\' | '"' | '$' | '`') {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    escaped
}

/// Simple sandbox names are safe as written; anything else gets quoted so a
/// name with spaces cannot split into two arguments.
fn shell_word(value: &str) -> String {
    if !value.is_empty()
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/'))
    {
        value.to_owned()
    } else {
        format!("\"{}\"", shell_double_quoted(value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_secret_list_from_the_cli() {
        let json = br#"{
          "secrets": [
            { "scope": "global", "type": "service", "name": "github", "secret": "(stored)" },
            { "scope": "test", "type": "service", "name": "nebius", "secret": "(stored)" }
          ],
          "custom_secrets": [],
          "shadowed_services": [],
          "env_only_count": 0
        }"#;

        assert_eq!(
            parse_secrets(json).unwrap(),
            vec![
                Secret::new("global", "github"),
                Secret::new("test", "nebius"),
            ]
        );
    }

    #[test]
    fn includes_custom_secrets_and_ignores_rows_without_a_name() {
        let json = br#"{
          "secrets": [{ "scope": "global", "name": "openai" }, {}],
          "custom_secrets": [{ "env_name": "MY_TOKEN", "host": "example.com" }]
        }"#;

        assert_eq!(
            parse_secrets(json).unwrap(),
            vec![
                Secret::new("global", "openai"),
                Secret::new("custom", "MY_TOKEN"),
            ]
        );
    }

    #[test]
    fn service_rows_carry_their_env_vars_and_api_domains() {
        let github = Secret::new("global", "github");
        assert_eq!(github.env_vars_label(), "GH_TOKEN, GITHUB_TOKEN");
        assert_eq!(
            github.api_domains,
            [
                "api.github.com",
                "github.com",
                "raw.githubusercontent.com",
                "gist.github.com",
                "copilot.github.com",
                "api.githubcopilot.com",
            ]
        );

        let groq = Secret::new("test", "groq");
        assert_eq!(groq.env_vars, ["GROQ_API_KEY"]);
        assert_eq!(groq.api_domains, ["api.groq.com"]);
    }

    #[test]
    fn a_custom_secret_uses_its_name_as_the_env_var() {
        let custom = Secret::new("custom", "MY_TOKEN");
        assert_eq!(custom.env_vars, ["MY_TOKEN"]);
        assert!(custom.api_domains.is_empty());
    }

    #[test]
    fn rejects_output_without_a_secret_list() {
        assert!(parse_secrets(br#"{"env_only_count": 7}"#).is_err());
        assert!(parse_secrets(b"not json").is_err());
    }

    #[test]
    fn existing_secret_lookup_reads_the_secrets_array() {
        assert!(parse_secret_exists(br#"{"secrets":[{"name":"github"}]}"#).unwrap());
        assert!(!parse_secret_exists(br#"{"secrets":[]}"#).unwrap());
        assert!(parse_secret_exists(br#"{"custom_secrets":[{"env_name":"TOKEN"}]}"#).is_err());
    }
}
