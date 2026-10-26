//! Local, user-owned per-agent configuration for BYOD mode.
//!
//! The file is the source of truth in `configMode: local`. Every apply entry
//! point uses [`effective`] so a restart, reconnect, periodic sync, or
//! controller push cannot silently replace it.

use std::{
    collections::BTreeMap,
    fmt,
    path::{Path, PathBuf},
};

use agentdesktop_core::config::{
    ClaudeCodeConfig, ClaudeDesktopConfig, CodexConfig, ConfigMode, CopilotConfig, DaemonConfig,
    GrokConfig, OpenCodeConfig, ProgramsConfig, VsCodeConfig,
};
use anyhow::{Context, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::secure_fs;

const MAX_BYTES: usize = 256 * 1024;
const MAX_DEPTH: usize = 32;

/// Persisted local configuration.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalConfigFile {
    #[serde(default)]
    pub revision: u64,
    /// `programs.<programKey>` values: `null` removes managed content, an
    /// object replaces that program's managed settings.
    #[serde(default)]
    pub programs: BTreeMap<String, Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub program_selection: Option<Value>,
}

/// Why a local configuration write was rejected.
#[derive(Debug)]
pub enum UpdateError {
    Conflict { current_revision: u64 },
    Invalid(anyhow::Error),
}

impl fmt::Display for UpdateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Conflict { current_revision } => {
                write!(
                    formatter,
                    "revision conflict; current revision is {current_revision}"
                )
            }
            Self::Invalid(error) => write!(formatter, "{error:#}"),
        }
    }
}

impl std::error::Error for UpdateError {}

impl From<anyhow::Error> for UpdateError {
    fn from(error: anyhow::Error) -> Self {
        Self::Invalid(error)
    }
}

/// The on-disk local configuration and its revision.
#[derive(Clone, Debug)]
pub struct LocalConfigStore {
    path: PathBuf,
    file: LocalConfigFile,
}

impl LocalConfigStore {
    /// Load the file, or start at revision 0 when it does not exist.
    pub fn load(path: PathBuf) -> anyhow::Result<Self> {
        let file = match std::fs::read_to_string(&path) {
            Ok(contents) => {
                let file: LocalConfigFile = serde_yaml::from_str(&contents)
                    .with_context(|| format!("parse local configuration {}", path.display()))?;
                validate_program_map(&file.programs)?;
                file
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                LocalConfigFile::default()
            }
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("read local configuration {}", path.display()));
            }
        };
        Ok(Self { path, file })
    }

    pub fn current(&self) -> &LocalConfigFile {
        &self.file
    }

    /// Replace the program map. `expected_revision` must match the current
    /// revision; a mismatch returns [`UpdateError::Conflict`].
    pub fn update(
        &mut self,
        expected_revision: u64,
        programs: BTreeMap<String, Value>,
        program_selection: Option<Value>,
    ) -> Result<u64, UpdateError> {
        if expected_revision != self.file.revision {
            return Err(UpdateError::Conflict {
                current_revision: self.file.revision,
            });
        }
        validate_program_map(&programs)?;
        if let Some(selection) = &program_selection {
            validate_value(selection, 0)?;
        }
        let next = LocalConfigFile {
            revision: self.file.revision.saturating_add(1),
            programs,
            program_selection,
        };
        let encoded = serde_yaml::to_string(&next).context("encode local configuration")?;
        let directory = self
            .path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        secure_fs::ensure_private_dir(directory)?;
        secure_fs::atomic_write(&self.path, encoded.as_bytes(), 0o600)?;
        self.file = next;
        Ok(self.file.revision)
    }

    /// Compute the configuration the reconciler should apply.
    ///
    /// In `managed` mode the base configuration wins and this file is ignored.
    /// In `local` mode the base program value is inherited when a key is
    /// absent, removed when it is `null`, and replaced when it is an object.
    pub fn effective(&self, base: &DaemonConfig) -> anyhow::Result<DaemonConfig> {
        let mut effective = base.clone();
        if base.config_mode != ConfigMode::Local {
            return Ok(effective);
        }
        validate_program_map(&self.file.programs)?;
        for (key, value) in &self.file.programs {
            apply_program(&mut effective.programs, key, value.clone())?;
        }
        if let Some(selection) = &self.file.program_selection {
            effective.program_selection =
                serde_json::from_value(selection.clone()).context("parse program selection")?;
        }
        Ok(effective)
    }
}

fn apply_program(programs: &mut ProgramsConfig, key: &str, value: Value) -> anyhow::Result<()> {
    macro_rules! apply {
        ($field:ident, $ty:ty) => {{
            programs.$field = if value.is_null() {
                None
            } else {
                Some(
                    serde_json::from_value::<$ty>(value)
                        .with_context(|| format!("parse local configuration for {key}"))?,
                )
            };
        }};
    }
    match key {
        "claudeCode" => apply!(claude_code, ClaudeCodeConfig),
        "claudeDesktop" => apply!(claude_desktop, ClaudeDesktopConfig),
        "codex" => apply!(codex, CodexConfig),
        "openCode" => apply!(open_code, OpenCodeConfig),
        "grok" => apply!(grok, GrokConfig),
        "copilot" => apply!(copilot, CopilotConfig),
        "vscode" => apply!(vscode, VsCodeConfig),
        other => anyhow::bail!("unknown program key {other:?}"),
    }
    Ok(())
}

fn validate_program_map(programs: &BTreeMap<String, Value>) -> anyhow::Result<()> {
    let encoded = serde_json::to_vec(programs).context("measure local configuration")?;
    ensure!(
        encoded.len() <= MAX_BYTES,
        "local configuration exceeds {MAX_BYTES} bytes"
    );
    for (key, value) in programs {
        ensure!(
            matches!(
                key.as_str(),
                "claudeCode"
                    | "claudeDesktop"
                    | "codex"
                    | "openCode"
                    | "grok"
                    | "copilot"
                    | "vscode"
            ),
            "unknown program key {key:?}"
        );
        ensure!(
            value.is_null() || value.is_object(),
            "program {key} must be null or an object"
        );
        validate_value(value, 0)?;
    }
    Ok(())
}

fn validate_value(value: &Value, depth: usize) -> anyhow::Result<()> {
    ensure!(
        depth <= MAX_DEPTH,
        "local configuration is nested too deeply"
    );
    match value {
        Value::Object(map) => {
            for (key, value) in map {
                ensure!(
                    !is_secret_key(key),
                    "secret-bearing key {key:?} is not allowed"
                );
                validate_value(value, depth + 1)?;
            }
        }
        Value::Array(items) => {
            for item in items {
                validate_value(item, depth + 1)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn is_secret_key(key: &str) -> bool {
    let normalized: String = key
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(|character| character.to_lowercase())
        .collect();
    matches!(
        normalized.as_str(),
        "apikey"
            | "apikeyhelper"
            | "token"
            | "accesstoken"
            | "refreshtoken"
            | "secret"
            | "password"
            | "authorization"
            | "headers"
            | "env"
            | "environment"
            | "credential"
            | "credentials"
            | "privatekey"
            | "clientsecret"
            | "cookie"
            | "session"
    )
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use agentdesktop_core::config::parse_daemon;
    use serde_json::json;
    use tempfile::tempdir;

    use super::{LocalConfigStore, UpdateError, is_secret_key};

    fn base() -> agentdesktop_core::config::DaemonConfig {
        parse_daemon(
            r#"
configMode: local
programs:
  claudeCode:
    theme: dark
  codex:
    managedConfig:
      approval_policy: never
"#,
        )
        .expect("base config")
    }

    #[test]
    fn absent_keys_inherit_base_null_removes_and_object_replaces() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("local-config.yaml");
        let mut store = LocalConfigStore::load(path.clone()).unwrap();
        let mut programs = BTreeMap::new();
        programs.insert("claudeCode".to_owned(), json!({"theme": "light"}));
        programs.insert("codex".to_owned(), json!(null));
        store.update(0, programs, None).unwrap();

        let effective = store.effective(&base()).unwrap();
        let claude = effective.programs.claude_code.expect("claude config");
        assert_eq!(claude.settings.get("theme"), Some(&json!("light")));
        assert!(effective.programs.codex.is_none());

        let reloaded = LocalConfigStore::load(path).unwrap();
        assert_eq!(reloaded.current().revision, 1);
        assert_eq!(reloaded.current().programs.get("codex"), Some(&json!(null)));
    }

    #[test]
    fn secret_bearing_keys_are_rejected() {
        assert!(is_secret_key("apiKey"));
        assert!(is_secret_key("api_key"));
        assert!(is_secret_key("Authorization"));
        assert!(is_secret_key("headers"));
        assert!(!is_secret_key("maxTokens"));
        assert!(!is_secret_key("theme"));
    }

    #[test]
    fn an_update_with_a_stale_revision_is_a_conflict() {
        let directory = tempdir().unwrap();
        let mut store = LocalConfigStore::load(directory.path().join("local.yaml")).unwrap();
        store.update(0, BTreeMap::new(), None).expect("first write");
        let error = store
            .update(0, BTreeMap::new(), None)
            .expect_err("stale revision");
        assert!(matches!(
            error,
            UpdateError::Conflict {
                current_revision: 1
            }
        ));
    }

    #[test]
    fn a_secret_value_is_rejected_before_persisting() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("local.yaml");
        let mut store = LocalConfigStore::load(path.clone()).unwrap();
        let mut programs = BTreeMap::new();
        programs.insert(
            "claudeCode".to_owned(),
            json!({"settings": {"apiKey": "x"}}),
        );
        assert!(store.update(0, programs, None).is_err());
        assert!(!path.exists());
    }
}
