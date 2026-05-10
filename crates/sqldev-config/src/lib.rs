//! `.sqldev.yml` config loader.
//!
//! Responsibilities:
//! - Discover `.sqldev.yml` walking up from the current directory.
//! - Parse the YAML document into a strongly typed [`Config`].
//! - Interpolate `${VAR}` and `${VAR:-default}` references against the
//!   process environment **before** YAML parsing.
//! - Select a named env block (defaulting to `default_env`).
//!
//! What this crate intentionally does **not** do:
//! - Touch secrets stores. Passwords come either inline (dev only) or via
//!   env-var interpolation.
//! - Open TDS connections. `sqldev-conn` does that; this crate hands it a
//!   ready-made [`ConnectOptions`].

#![forbid(unsafe_code)]

mod interpolate;
mod loader;
mod model;

pub use interpolate::{InterpolateError, interpolate};
pub use loader::{ConfigLoadError, discover, load, load_from};
pub use model::{AuthBlock, Config, EnvBlock};

use sqldev_conn::{AuthOptions, ConnectOptions};

/// Convert a config env block to a [`ConnectOptions`] suitable for
/// `sqldev_conn::connect`. Mostly a field-by-field copy with the auth enum
/// rewritten to the `sqldev-conn` shape.
#[must_use]
pub fn env_to_connect_options(env: &EnvBlock) -> ConnectOptions {
    ConnectOptions {
        host: env.host.clone(),
        port: env.port,
        database: env.database.clone(),
        auth: match &env.auth {
            AuthBlock::Sql { user, password } => AuthOptions::Sql {
                user: user.clone(),
                password: password.clone(),
            },
        },
        trust_server_certificate: env.trust_server_certificate,
    }
}
