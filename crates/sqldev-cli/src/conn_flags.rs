//! Connection-options CLI flags shared across subcommands.
//!
//! Until `.sqldev.yml` parsing lands (M1.3), these flags ARE the connection
//! source. Once the config loader exists, this module will fall back to a
//! YAML env block when flags are omitted.

use clap::Args as ClapArgs;
use sqldev_conn::{AuthOptions, ConnectOptions};

#[derive(ClapArgs, Debug, Clone)]
pub struct ConnectionFlags {
    /// Server host (without port).
    #[arg(long, env = "SQLDEV_HOST", default_value = "localhost")]
    pub host: String,

    /// TDS port.
    #[arg(long, env = "SQLDEV_PORT", default_value_t = 1433)]
    pub port: u16,

    /// Database name.
    #[arg(long, env = "SQLDEV_DATABASE")]
    pub database: String,

    /// SQL login username.
    #[arg(long, env = "SQLDEV_USER", default_value = "SA")]
    pub user: String,

    /// SQL login password.
    #[arg(long, env = "SQLDEV_PASSWORD")]
    pub password: String,

    /// Trust the server certificate without validation. Convenient for
    /// local dev containers; do not enable on production env blocks.
    #[arg(long, env = "SQLDEV_TRUST_CERT", default_value_t = false)]
    pub trust_cert: bool,
}

impl ConnectionFlags {
    pub fn to_options(&self) -> ConnectOptions {
        ConnectOptions {
            host: self.host.clone(),
            port: self.port,
            database: self.database.clone(),
            auth: AuthOptions::Sql {
                user: self.user.clone(),
                password: self.password.clone(),
            },
            trust_server_certificate: self.trust_cert,
        }
    }
}
