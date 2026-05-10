//! File discovery + parse for `.sqldev.yml`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::interpolate::{InterpolateError, interpolate};
use crate::model::Config;

/// Standard config filename. We only look for one shape so users don't
/// have to wonder which extension we accept.
pub const CONFIG_FILE_NAME: &str = ".sqldev.yml";

#[derive(Debug, thiserror::Error)]
pub enum ConfigLoadError {
    #[error("config file not found: {0}")]
    NotFound(PathBuf),
    #[error("read config file `{path}`: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("interpolate config file `{path}`: {source}")]
    Interpolate {
        path: PathBuf,
        #[source]
        source: InterpolateError,
    },
    #[error("parse config file `{path}`: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: serde_yaml::Error,
    },
    #[error("get current directory: {0}")]
    Cwd(#[source] std::io::Error),
}

/// Walk up from `start` (or CWD if `None`) looking for `.sqldev.yml`.
/// Returns `Ok(None)` if nothing is found before hitting the filesystem
/// root, so callers can decide whether the lookup was required.
pub fn discover(start: Option<&Path>) -> Result<Option<PathBuf>, ConfigLoadError> {
    let mut cur = match start {
        Some(p) => p.to_path_buf(),
        None => std::env::current_dir().map_err(ConfigLoadError::Cwd)?,
    };
    loop {
        let candidate = cur.join(CONFIG_FILE_NAME);
        if candidate.exists() {
            return Ok(Some(candidate));
        }
        if !cur.pop() {
            return Ok(None);
        }
    }
}

/// Load the nearest `.sqldev.yml` (walking up from CWD). Returns `Ok(None)`
/// when no config file exists, which is a legitimate state — sqldev still
/// works with CLI flags only.
pub fn load() -> Result<Option<(PathBuf, Config)>, ConfigLoadError> {
    match discover(None)? {
        Some(path) => {
            let cfg = load_from(&path)?;
            Ok(Some((path, cfg)))
        }
        None => Ok(None),
    }
}

/// Load and parse a specific config file path. Used both by [`load`] and
/// when the caller passes an explicit `--config <path>` flag.
pub fn load_from(path: &Path) -> Result<Config, ConfigLoadError> {
    if !path.exists() {
        return Err(ConfigLoadError::NotFound(path.to_path_buf()));
    }
    let raw = std::fs::read_to_string(path).map_err(|e| ConfigLoadError::Io {
        path: path.to_path_buf(),
        source: e,
    })?;
    let interpolated =
        interpolate(&raw, &HashMap::new()).map_err(|e| ConfigLoadError::Interpolate {
            path: path.to_path_buf(),
            source: e,
        })?;
    let cfg: Config = serde_yaml::from_str(&interpolated).map_err(|e| ConfigLoadError::Parse {
        path: path.to_path_buf(),
        source: e,
    })?;
    Ok(cfg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_tmp(contents: &str) -> tempfile::NamedTempFile {
        let mut f = tempfile::Builder::new().suffix(".yml").tempfile().unwrap();
        f.write_all(contents.as_bytes()).unwrap();
        f
    }

    #[test]
    fn parse_minimal() {
        // SAFETY: tests are single-threaded under cargo test --test-threads=1
        // when env vars are involved; here we use a static value so the
        // process env doesn't matter.
        let f = write_tmp(
            r#"
default_env: dev
envs:
  dev:
    host: localhost
    port: 1433
    database: app_dev
    auth:
      kind: sql
      user: sa
      password: hunter2
    trust_server_certificate: true
"#,
        );
        let cfg = load_from(f.path()).unwrap();
        assert_eq!(cfg.default_env.as_deref(), Some("dev"));
        let (name, env) = cfg.select_env(None).unwrap();
        assert_eq!(name, "dev");
        assert_eq!(env.host, "localhost");
        assert!(env.trust_server_certificate);
        assert!(!env.protected);
    }

    #[test]
    fn unknown_field_rejected() {
        let f = write_tmp(
            r#"
envs:
  dev:
    host: x
    database: y
    auth: { kind: sql, user: u, password: p }
    bogus: 1
"#,
        );
        let err = load_from(f.path()).unwrap_err();
        matches!(err, ConfigLoadError::Parse { .. });
    }

    #[test]
    fn select_env_explicit() {
        let f = write_tmp(
            r#"
default_env: dev
envs:
  dev: { host: a, database: a, auth: { kind: sql, user: u, password: p } }
  prod: { host: b, database: b, auth: { kind: sql, user: u, password: p }, protected: true }
"#,
        );
        let cfg = load_from(f.path()).unwrap();
        let (name, env) = cfg.select_env(Some("prod")).unwrap();
        assert_eq!(name, "prod");
        assert!(env.protected);
    }

    #[test]
    fn select_env_unknown_errors() {
        let f = write_tmp(
            r#"
envs:
  dev: { host: a, database: a, auth: { kind: sql, user: u, password: p } }
"#,
        );
        let cfg = load_from(f.path()).unwrap();
        assert!(cfg.select_env(Some("nope")).is_err());
    }
}
