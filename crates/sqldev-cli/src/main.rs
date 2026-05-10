//! `sqldev` command-line entrypoint.
//!
//! M1 status: this binary currently exposes the introspection path that was
//! validated in the M0 spike, plus a thin `query` one-shot. Subcommands
//! `init`, `migrate`, `diff`, `explain`, and the REPL land in subsequent
//! M1 / M2 milestones.

#![forbid(unsafe_code)]

mod cmd_introspect;
mod cmd_query;
mod conn_flags;

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
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Walk the system catalog and emit the schema graph as JSON.
    Introspect(cmd_introspect::Args),
    /// Execute a one-shot T-SQL statement.
    Query(cmd_query::Args),
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
    match cli.cmd {
        Cmd::Introspect(args) => cmd_introspect::run(args).await,
        Cmd::Query(args) => cmd_query::run(args).await,
    }
}
