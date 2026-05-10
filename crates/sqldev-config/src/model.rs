//! Strongly-typed `.sqldev.yml` document.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Top-level document.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Name of the env block selected when `--env` is omitted. If absent and
    /// only one env exists, that one is used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_env: Option<String>,

    /// Map of env name → env block. Order is alphabetical on serialization.
    pub envs: BTreeMap<String, EnvBlock>,
}

/// One named environment (e.g. `dev`, `staging`, `prod`).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EnvBlock {
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    pub database: String,
    pub auth: AuthBlock,

    /// Trust the server certificate without validation. Convenient for
    /// local dev containers; rejected when `protected: true`.
    #[serde(default)]
    pub trust_server_certificate: bool,

    /// When true, destructive commands (`migrate up`, `migrate down`,
    /// `apply`, …) require an explicit `--confirm <env>` flag.
    #[serde(default)]
    pub protected: bool,
}

fn default_port() -> u16 {
    1433
}

/// Auth method discriminated by the `kind` tag.
///
/// Only SQL auth is supported in M1.3. The Entra ladder
/// (`entra-interactive`, `entra-managed-identity`, `entra-service-principal`)
/// lands together with M1.2 so we don't ship half-implemented variants.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum AuthBlock {
    Sql { user: String, password: String },
}

impl Config {
    /// Resolve which env block to use given an optional explicit selector.
    ///
    /// Precedence:
    /// 1. `requested` (e.g. CLI `--env <name>`)
    /// 2. `self.default_env`
    /// 3. The single env in `self.envs`, if exactly one is defined.
    pub fn select_env(&self, requested: Option<&str>) -> Result<(&str, &EnvBlock), SelectError> {
        let name = if let Some(r) = requested {
            r.to_string()
        } else if let Some(d) = &self.default_env {
            d.clone()
        } else if self.envs.len() == 1 {
            self.envs.keys().next().unwrap().clone()
        } else {
            return Err(SelectError::Ambiguous);
        };
        let block = self
            .envs
            .get(&name)
            .ok_or(SelectError::Unknown(name.clone()))?;
        // Borrow the key string from the map so the lifetime is tied to &self.
        let key = self.envs.get_key_value(&name).unwrap().0.as_str();
        Ok((key, block))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SelectError {
    #[error("no env selected and no `default_env` is set; pass --env <name>")]
    Ambiguous,
    #[error("env `{0}` is not defined in the config file")]
    Unknown(String),
}
