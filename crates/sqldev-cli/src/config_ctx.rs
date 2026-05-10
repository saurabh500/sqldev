//! Global config-file resolution: turns the top-level `--config` /
//! `--env` flags into an optional `(name, EnvBlock)` pair the per-subcommand
//! `ConnectionFlags` resolver can consume.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use sqldev_config::{Config, EnvBlock};

/// Resolved config state available to every subcommand.
pub struct ConfigContext {
    /// `None` when no config file was discovered (and none was explicitly
    /// requested). The CLI still works in this mode using flags + env vars.
    pub config: Option<Config>,
    pub source: Option<PathBuf>,
    /// Name of the env block selected (when a config exists and an env
    /// could be picked).
    pub selected_env: Option<String>,
}

impl ConfigContext {
    /// Borrow the selected env block (if any).
    pub fn env_block(&self) -> Option<(&str, &EnvBlock)> {
        match (&self.config, &self.selected_env) {
            (Some(cfg), Some(name)) => cfg.envs.get_key_value(name).map(|(k, v)| (k.as_str(), v)),
            _ => None,
        }
    }
}

/// Load a config file (explicit path or auto-discovered) and resolve the
/// requested env block.
///
/// `requested_env` is the value of the global `--env` flag.
///
/// `require_config` makes a missing config file an error rather than a
/// silent `Ok(None)`. Useful for `sqldev config show`.
pub fn load(
    explicit_path: Option<&Path>,
    requested_env: Option<&str>,
    require_config: bool,
) -> Result<ConfigContext> {
    let loaded = if let Some(p) = explicit_path {
        let cfg = sqldev_config::load_from(p).context("load config file")?;
        Some((p.to_path_buf(), cfg))
    } else {
        sqldev_config::load().context("discover/load .sqldev.yml")?
    };

    let Some((source, cfg)) = loaded else {
        if require_config {
            anyhow::bail!(
                "no .sqldev.yml found (run from a directory containing one, or pass --config <path>)"
            );
        }
        if requested_env.is_some() {
            anyhow::bail!(
                "--env was passed but no .sqldev.yml was found (run from a directory containing one, or pass --config <path>)"
            );
        }
        return Ok(ConfigContext {
            config: None,
            source: None,
            selected_env: None,
        });
    };

    // Resolve env name even when no env was explicitly requested, so that
    // subcommands automatically pick up the default block.
    let selected_env = match cfg.select_env(requested_env) {
        Ok((name, _)) => Some(name.to_string()),
        // If the user didn't ask for an env and the file is ambiguous, just
        // proceed with no env — flags are still required from the user.
        Err(_) if requested_env.is_none() => None,
        Err(e) => return Err(anyhow::Error::new(e)).context("select env block"),
    };

    Ok(ConfigContext {
        config: Some(cfg),
        source: Some(source),
        selected_env,
    })
}
