//! Reading the secret inventory shown in the Secrets tab.

use serde_json::Value;

use super::sandbox::{Sandbox, fetch_sandboxes, json_text};
use super::secret_form::{CustomSecretForm, GLOBAL_SCOPE, SecretForm, is_global_scope};
use super::{
    App, CONFIRM_DIALOG, Mode, PendingAction, PendingCommand, SET_CUSTOM_SECRET_DIALOG,
    SET_SECRET_DIALOG, service_info,
};

/// One row of a scoped `sbx secret ls --json`, reduced to the fields the
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

/// `sbx secret ls -g --json` for the global store, or
/// `sbx secret ls --sandbox <name> --json` for one sandbox.
fn secret_list_command(scope: Option<&str>) -> PendingCommand {
    let mut args = vec!["secret", "ls"];
    match scope {
        None => args.extend(["-g", "--json"]),
        Some(sandbox) => args.extend(["--sandbox", sandbox, "--json"]),
    }
    PendingCommand::new("sbx", &args)
}

/// Run one scope's listing and label every row with that scope. The CLI calls
/// global secrets "global", while the table and the Set dialog use
/// `"(global)"`, so the requested label wins over the JSON field.
fn fetch_secret_scope(scope: Option<&str>, label: &str) -> Result<Vec<Secret>, String> {
    let command = secret_list_command(scope);
    let output = command.run().map_err(|error| error.to_string())?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        let detail = if stderr.is_empty() {
            format!("exited with {}", output.status)
        } else {
            stderr
        };
        return Err(format!("{}: {detail}", command.preview()));
    }
    parse_secrets(&output.stdout, label)
}

/// List stored secrets across the global store and every sandbox. The CLI
/// reports actual values as `(stored)`, so nothing sensitive is kept in state.
#[allow(dead_code)] // Public entry point; the dashboard uses `fetch_secrets_for`.
pub fn fetch_secrets() -> Result<Vec<Secret>, String> {
    let sandboxes = fetch_sandboxes().unwrap_or_default();
    fetch_secrets_for(&sandboxes)
}

/// The same listing for callers that already know their sandboxes.
pub fn fetch_secrets_for(sandboxes: &[Sandbox]) -> Result<Vec<Secret>, String> {
    let mut secrets = fetch_secret_scope(None, GLOBAL_SCOPE)?;
    for sandbox in sandboxes {
        secrets.extend(fetch_secret_scope(Some(&sandbox.name), &sandbox.name)?);
    }
    Ok(secrets)
}

/// Both `secrets` and `custom_secrets` from one scoped listing. The latter has
/// no service of its own, but it still belongs to the scope being queried.
fn parse_secrets(bytes: &[u8], scope: &str) -> Result<Vec<Secret>, String> {
    let value: Value = serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
    let mut secrets = Vec::new();

    for field in ["secrets", "custom_secrets"] {
        let Some(rows) = value.get(field).and_then(Value::as_array) else {
            continue;
        };
        for row in rows {
            let Some(name) = json_text(row, &["name", "service", "env_name"]) else {
                continue;
            };
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

    /// Open the Set custom secret dialog with one empty host row. Its scope
    /// options are the same snapshot as Set secret: `(global)`, then every
    /// sandbox, with a sandbox scope becoming `--sandbox` on the CLI.
    pub fn set_custom_secret(&mut self) {
        if self.mode() != Mode::Normal {
            return;
        }
        self.custom_secret_form = Some(CustomSecretForm::new(&self.sandboxes));
        let _ = self.modals.open(SET_CUSTOM_SECRET_DIALOG, &mut self.focus);
    }

    pub fn cancel_custom_secret(&mut self) {
        self.close_modal(SET_CUSTOM_SECRET_DIALOG);
        self.custom_secret_form = None;
        self.notify_info("Set custom secret cancelled.", None);
    }

    pub fn set_custom_env_key(&mut self, state: ratcn::InputState) {
        if let Some(form) = self.custom_secret_form.as_mut() {
            form.set_env_key(state);
        }
    }

    pub fn set_custom_secret_value(&mut self, state: ratcn::InputState) {
        if let Some(form) = self.custom_secret_form.as_mut() {
            form.set_secret(state);
        }
    }

    pub fn set_custom_scope_open(&mut self, open: bool) {
        if let Some(form) = self.custom_secret_form.as_mut() {
            form.set_scope_open(open);
        }
    }

    pub fn set_custom_scope_focus(&mut self, index: usize) {
        if let Some(form) = self.custom_secret_form.as_mut() {
            form.set_scope_focus(index);
        }
    }

    pub fn set_custom_scope(&mut self, index: usize) {
        if let Some(form) = self.custom_secret_form.as_mut() {
            form.set_scope_index(index);
        }
    }

    pub fn set_custom_host(&mut self, index: usize, state: ratcn::InputState) {
        if let Some(form) = self.custom_secret_form.as_mut() {
            form.set_host(index, state);
        }
    }

    pub fn add_custom_host(&mut self) {
        if let Some(form) = self.custom_secret_form.as_mut() {
            form.add_host();
        }
    }

    /// Validate and submit the custom form. The value travels as a plain CLI
    /// argument, so no shell quoting is needed around the secret.
    pub fn submit_custom_secret(&mut self) {
        if self.mode() != Mode::SetCustomSecret {
            return;
        }
        let Some(form) = self.custom_secret_form.as_mut() else {
            return;
        };
        form.mark_attempted();
        let env_key = form.env_key().value().trim().to_owned();
        let scope = form.scope().map(str::to_owned);
        let secret = form.secret().value().trim().to_owned();
        let hosts: Vec<String> = form
            .hosts()
            .iter()
            .map(|host| host.value().trim().to_owned())
            .collect();

        if scope.is_none() {
            self.notify_error("Invalid custom secret", "Choose a scope.");
            return;
        }
        if env_key.is_empty() {
            self.notify_error("Invalid custom secret", "Enter an env key.");
            return;
        }
        if secret.is_empty() {
            self.notify_error("Invalid custom secret", "Enter a secret value.");
            return;
        }
        if hosts.iter().any(String::is_empty) {
            self.notify_error("Invalid custom secret", "Enter a value for every host.");
            return;
        }

        let scope = scope.as_deref().unwrap_or(GLOBAL_SCOPE);
        let pending = Self::set_custom_secret_command(&env_key, scope, &hosts, &secret);
        let success = format!("Set custom secret '{env_key}'.");

        #[cfg(test)]
        {
            self.set_for_test = Some(pending.preview());
            self.close_modal(SET_CUSTOM_SECRET_DIALOG);
            self.custom_secret_form = None;
            self.notify_success(success, None);
        }

        #[cfg(not(test))]
        {
            self.notify_info("Setting custom secret...", None);
            self.jobs
                .spawn(move || super::workers::JobResult::CustomSecretSet {
                    success,
                    result: pending.run(),
                });
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

        #[cfg(test)]
        {
            let exists = self.stored_secret_exists(&service, &scope).unwrap_or(false);
            self.finish_secret_lookup(&service, &scope, Ok(exists));
        }

        #[cfg(not(test))]
        {
            // The overwrite lookup also shells out, so it runs on a worker and
            // the dialog stays usable while `sbx secret ls` is in flight.
            let lookup_service = service.clone();
            let lookup_scope = scope.clone();
            let check_service = lookup_service.clone();
            let check_scope = lookup_scope.clone();
            self.notify_info("Checking stored secret...", None);
            self.jobs
                .spawn(move || super::workers::JobResult::SecretExists {
                    service: lookup_service,
                    scope: lookup_scope,
                    result: Self::stored_secret_exists(&check_service, &check_scope),
                });
        }
    }

    /// Apply the overwrite lookup on the UI thread. A late answer is ignored
    /// if the dialog was closed or the user changed the form in the meantime.
    pub(super) fn finish_secret_lookup(
        &mut self,
        service: &str,
        scope: &str,
        result: Result<bool, String>,
    ) {
        let still_open = self
            .secret_form
            .as_ref()
            .is_some_and(|form| form.service() == Some(service) && form.scope() == Some(scope));
        if !still_open {
            return;
        }
        match result {
            Ok(true) => {
                self.pending_action = Some(PendingAction::OverwriteSecret(
                    service.to_owned(),
                    scope.to_owned(),
                ));
                let _ = self.modals.open(CONFIRM_DIALOG, &mut self.focus);
            }
            Ok(false) => self.finish_set_secret(service, scope),
            Err(error) => self.notify_error("Could not check secret", error),
        }
    }

    /// `sbx secret ls --sandbox <scope> --service <service> --format json`.
    /// The global store has no sandbox, so it uses `--global` instead.
    pub(super) fn secret_lookup_command(service: &str, scope: &str) -> PendingCommand {
        let mut args = vec!["secret", "ls"];
        if is_global_scope(scope) {
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
    fn stored_secret_exists(service: &str, scope: &str) -> Result<bool, String> {
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
        let target = if is_global_scope(scope) {
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
            let set_service = service.to_owned();
            let set_scope = scope.to_owned();
            self.notify_info("Setting secret...", None);
            self.jobs
                .spawn(move || super::workers::JobResult::SecretSet {
                    service: set_service,
                    scope: set_scope,
                    success,
                    result: pending.run(),
                });
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

    /// `sbx secret set-custom [--sandbox <scope>] --host <host>... --env <key> --value <secret>`.
    /// The `(global)` scope omits `--sandbox`, as in Set secret. Arguments are
    /// passed directly, so values with spaces or shell metachars stay a single
    /// argument instead of being reinterpreted by a shell.
    fn set_custom_secret_command(
        env_key: &str,
        scope: &str,
        hosts: &[String],
        secret: &str,
    ) -> PendingCommand {
        let mut args = vec!["secret".to_owned(), "set-custom".to_owned()];
        if !is_global_scope(scope) {
            args.extend(["--sandbox".to_owned(), scope.to_owned()]);
        }
        for host in hosts {
            args.push("--host".to_owned());
            args.push(host.clone());
        }
        args.extend([
            "--env".to_owned(),
            env_key.to_owned(),
            "--value".to_owned(),
            secret.to_owned(),
        ]);
        let borrowed: Vec<&str> = args.iter().map(String::as_str).collect();
        PendingCommand::new("sbx", &borrowed)
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
    fn parses_the_global_secret_list_from_the_cli() {
        let json = br#"{
          "secrets": [
            { "scope": "global", "type": "service", "name": "github", "secret": "(stored)" },
            { "scope": "global", "type": "service", "name": "nebius", "secret": "(stored)" }
          ],
          "custom_secrets": [],
          "shadowed_services": [],
          "env_only_count": 0
        }"#;

        assert_eq!(
            parse_secrets(json, GLOBAL_SCOPE).unwrap(),
            vec![
                Secret::new(GLOBAL_SCOPE, "github"),
                Secret::new(GLOBAL_SCOPE, "nebius"),
            ]
        );
    }

    #[test]
    fn a_scoped_listing_labels_every_row_with_the_sandbox_name() {
        let json = br#"{
          "secrets": [
            { "scope": "global", "name": "github" },
            { "scope": "test", "name": "nebius" }
          ],
          "custom_secrets": [{ "env_name": "MY_TOKEN" }, {}]
        }"#;

        assert_eq!(
            parse_secrets(json, "test").unwrap(),
            vec![
                Secret::new("test", "github"),
                Secret::new("test", "nebius"),
                Secret::new("test", "MY_TOKEN"),
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
            parse_secrets(json, GLOBAL_SCOPE).unwrap(),
            vec![
                Secret::new(GLOBAL_SCOPE, "openai"),
                Secret::new(GLOBAL_SCOPE, "MY_TOKEN"),
            ]
        );
    }

    #[test]
    fn list_commands_target_one_scope_at_a_time() {
        assert_eq!(
            secret_list_command(None).preview(),
            "sbx secret ls -g --json"
        );
        assert_eq!(
            secret_list_command(Some("alpha")).preview(),
            "sbx secret ls --sandbox alpha --json"
        );
        assert_eq!(
            secret_list_command(Some("my sandbox")).preview(),
            "sbx secret ls --sandbox 'my sandbox' --json"
        );
    }

    #[test]
    fn service_rows_carry_their_env_vars_and_api_domains() {
        let github = Secret::new(GLOBAL_SCOPE, "github");
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
        assert!(parse_secrets(br#"{"env_only_count": 7}"#, GLOBAL_SCOPE).is_err());
        assert!(parse_secrets(b"not json", GLOBAL_SCOPE).is_err());
    }

    #[test]
    fn existing_secret_lookup_reads_the_secrets_array() {
        assert!(parse_secret_exists(br#"{"secrets":[{"name":"github"}]}"#).unwrap());
        assert!(!parse_secret_exists(br#"{"secrets":[]}"#).unwrap());
        assert!(parse_secret_exists(br#"{"custom_secrets":[{"env_name":"TOKEN"}]}"#).is_err());
    }
}
