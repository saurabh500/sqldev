//! `sqldev` command-line entrypoint.

#![forbid(unsafe_code)]

mod cmd_config;
mod cmd_introspect;
mod cmd_query;
mod config_ctx;
mod conn_flags;
mod output;

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};
use tracing_subscriber::EnvFilter;

#[derive(Parser, Debug)]
#[command(
    name = "sqldev",
    version,
    about = "A developer-friendly SQL Server CLI.",
    propagate_version = true
)]
struct Cli {
    /// Path to a `.sqldev.yml`. When omitted, sqldev walks up from the
    /// current directory looking for one.
    #[arg(long, global = true)]
    config: Option<PathBuf>,

    /// Name of the env block to use. Defaults to the file's `default_env`.
    #[arg(long, global = true)]
    env: Option<String>,

    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Walk the system catalog and emit the schema graph as JSON.
    Introspect(cmd_introspect::Args),
    /// Execute a one-shot T-SQL statement.
    Query(cmd_query::Args),
    /// Inspect resolved `.sqldev.yml` configuration.
    Config(cmd_config::Args),
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_env("SQLDEV_LOG").unwrap_or_else(|_| EnvFilter::new("warn")),
        )
        .with_target(false)
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();

    // `config show` requires a config file; everything else treats it as
    // optional.
    let require_config = matches!(cli.cmd, Cmd::Config(_));
    let ctx = config_ctx::load(cli.config.as_deref(), cli.env.as_deref(), require_config)?;

    match cli.cmd {
        Cmd::Introspect(args) => cmd_introspect::run(args, &ctx).await,
        Cmd::Query(args) => cmd_query::run(args, &ctx).await,
        Cmd::Config(args) => cmd_config::run(args, &ctx),
    }
}
