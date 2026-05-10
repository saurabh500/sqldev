//! Connection-options CLI flags shared across subcommands.
//!
//! Resolution precedence (highest wins):
//! 1. CLI flag explicitly passed by the user.
//! 2. `.sqldev.yml` env block selected by `--env <name>` (or `default_env`).
//! 3. `SQLDEV_*` process env var.
//! 4. Hard-coded defaults (`localhost`, port `1433`, user `SA`).
//!
//! When no env block is selected, only steps 1, 3, 4 apply — i.e. the CLI
//! still works without a config file.

use std::env;

use clap::Args as ClapArgs;
use sqldev_config::{AuthBlock, EnvBlock};
use sqldev_conn::{AuthOptions, ConnectOptions};

#[derive(ClapArgs, Debug, Clone)]
pub struct ConnectionFlags {
    /// Server host (without port).
    #[arg(long)]
    pub host: Option<String>,

    /// TDS port.
    #[arg(long)]
    pub port: Option<u16>,

    /// Database name.
    #[arg(long)]
    pub database: Option<String>,

    /// SQL login username.
    #[arg(long)]
    pub user: Option<String>,

    /// SQL login password.
    #[arg(long)]
    pub password: Option<String>,

    /// Trust the server certificate without validation. Convenient for
    /// local dev containers; rejected when the env block has
    /// `protected: true`.
    #[arg(long)]
    pub trust_cert: Option<bool>,
}

#[derive(Debug, thiserror::Error)]
pub enum ResolveError {
    #[error("missing required connection field: --{0} (also no env block / SQLDEV_{1})")]
    Missing(&'static str, &'static str),
    #[error(
        "env block `{0}` is marked `protected: true` but --trust-cert is set; remove --trust-cert or pick a different env"
    )]
    ProtectedTrustCert(String),
}

impl ConnectionFlags {
    /// Resolve the flags into a [`ConnectOptions`], optionally falling back
    /// to a `.sqldev.yml` env block.
    pub fn resolve(
        &self,
        env_block: Option<(&str, &EnvBlock)>,
    ) -> Result<ConnectOptions, ResolveError> {
        let host = self
            .host
            .clone()
            .or_else(|| env_block.map(|(_, e)| e.host.clone()))
            .or_else(|| env::var("SQLDEV_HOST").ok())
            .unwrap_or_else(|| "localhost".to_string());

        let port = self
            .port
            .or_else(|| env_block.map(|(_, e)| e.port))
            .or_else(|| env::var("SQLDEV_PORT").ok().and_then(|s| s.parse().ok()))
            .unwrap_or(1433);

        let database = self
            .database
            .clone()
            .or_else(|| env_block.map(|(_, e)| e.database.clone()))
            .or_else(|| env::var("SQLDEV_DATABASE").ok())
            .ok_or(ResolveError::Missing("database", "DATABASE"))?;

        let auth = self.resolve_auth(env_block.map(|(_, e)| &e.auth))?;

        let trust_server_certificate = self
            .trust_cert
            .or_else(|| env_block.map(|(_, e)| e.trust_server_certificate))
            .or_else(|| {
                env::var("SQLDEV_TRUST_CERT")
                    .ok()
                    .map(|s| matches!(s.as_str(), "1" | "true" | "TRUE" | "True"))
            })
            .unwrap_or(false);

        if trust_server_certificate
            && let Some((name, env)) = env_block
            && env.protected
        {
            return Err(ResolveError::ProtectedTrustCert(name.to_string()));
        }

        Ok(ConnectOptions {
            host,
            port,
            database,
            auth,
            trust_server_certificate,
        })
    }

    fn resolve_auth(&self, env_auth: Option<&AuthBlock>) -> Result<AuthOptions, ResolveError> {
        // If the user passed any CLI cred, treat the result as SQL auth and
        // overlay missing fields from the env block / process env / default.
        if self.user.is_some() || self.password.is_some() {
            let user = self
                .user
                .clone()
                .or_else(|| env_auth.map(|AuthBlock::Sql { user, .. }| user.clone()))
                .or_else(|| env::var("SQLDEV_USER").ok())
                .unwrap_or_else(|| "SA".to_string());
            let password = self
                .password
                .clone()
                .or_else(|| env_auth.map(|AuthBlock::Sql { password, .. }| password.clone()))
                .or_else(|| env::var("SQLDEV_PASSWORD").ok())
                .ok_or(ResolveError::Missing("password", "PASSWORD"))?;
            return Ok(AuthOptions::Sql { user, password });
        }
        if let Some(AuthBlock::Sql { user, password }) = env_auth {
            return Ok(AuthOptions::Sql {
                user: user.clone(),
                password: password.clone(),
            });
        }
        let user = env::var("SQLDEV_USER").unwrap_or_else(|_| "SA".to_string());
        let password = env::var("SQLDEV_PASSWORD")
            .map_err(|_| ResolveError::Missing("password", "PASSWORD"))?;
        Ok(AuthOptions::Sql { user, password })
    }
}
