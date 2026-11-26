//! Reading the secret inventory shown in the Secrets tab.

use std::process::Command as ProcessCommand;

use serde_json::Value;

use super::sandbox::json_text;

/// One row of `sbx secret ls --format json`, reduced to the fields the
/// dashboard table shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Secret {
    pub scope: String,
    pub name: String,
}

impl Secret {
    pub fn new(scope: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            scope: scope.into(),
            name: name.into(),
        }
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
    fn rejects_output_without_a_secret_list() {
        assert!(parse_secrets(br#"{"env_only_count": 7}"#).is_err());
        assert!(parse_secrets(b"not json").is_err());
    }
}
