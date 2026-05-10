//! TDS connection layer.
//!
//! Today this is a thin wrapper over [`mssql_tiberius_bridge`] that
//! standardizes connection options and centralizes the auth-method matrix.
//! Pooling, retry, and the full Entra auth ladder land in M1.2.

#![forbid(unsafe_code)]

use mssql_tiberius_bridge::{AuthMethod, Client, Config};
use serde::Deserialize;
use sqldev_core::{Error, Result};

/// Minimum knobs required to open a TDS connection.
///
/// Mirrors the shape of a `.sqldev.yml` env block so it can be deserialized
/// directly once that loader lands.
#[derive(Debug, Clone, Deserialize)]
pub struct ConnectOptions {
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    pub database: String,
    pub auth: AuthOptions,
    /// Trust the server certificate without validation. Convenient for local
    /// dev containers; **never** set on a production env block.
    #[serde(default)]
    pub trust_server_certificate: bool,
}

fn default_port() -> u16 {
    1433
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum AuthOptions {
    /// SQL login.
    Sql { user: String, password: String },
    // Entra interactive / managed-identity / service-principal land in M1.2;
    // intentionally omitted so we don't ship half-implemented variants.
}

/// Open a connection.
pub async fn connect(opts: &ConnectOptions) -> Result<Client> {
    let mut cfg = Config::new();
    cfg.host(&opts.host)
        .port(opts.port)
        .database(&opts.database);
    match &opts.auth {
        AuthOptions::Sql { user, password } => {
            cfg.authentication(AuthMethod::sql_server(user, password));
        }
    }
    if opts.trust_server_certificate {
        cfg.trust_cert();
    }
    Client::connect(&cfg)
        .await
        .map_err(|e| Error::Connect(e.to_string()))
}
