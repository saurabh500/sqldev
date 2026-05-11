//! `sqldev` command-line entrypoint.

#![forbid(unsafe_code)]

mod cmd_config;
mod cmd_diff;
mod cmd_explain;
mod cmd_init;
mod cmd_introspect;
mod cmd_migrate;
mod cmd_query;
mod cmd_seed;
mod cmd_snapshot;
mod cmd_telemetry;
mod config_ctx;
mod conn_flags;
mod output;
mod repl;
mod telemetry;

use std::path::PathBuf;
use std::time::Instant;

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
    /// Bootstrap a new sqldev project (`.sqldev.yml`, baseline migration,
    /// `migrations/` / `seeds/` / `models/` folders).
    Init(cmd_init::Args),
    /// Apply or roll back versioned T-SQL migrations.
    Migrate(cmd_migrate::Args),
    /// Analyze a query plan for common anti-patterns.
    Explain(cmd_explain::Args),
    /// Diff two schema graphs and emit a T-SQL migration script.
    Diff(cmd_diff::Args),
    /// Capture the live schema graph as JSON for offline diffing in CI.
    Snapshot(cmd_snapshot::Args),
    /// Populate tables with type-aware fake data from YAML seed files.
    Seed(cmd_seed::Args),
    /// Manage anonymous usage telemetry (opt-in, off by default).
    Telemetry(cmd_telemetry::Args),
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

    telemetry::install_panic_hook();

    let cli = Cli::parse();

    // `config show` requires a config file; everything else treats it as
    // optional.
    let require_config = matches!(cli.cmd, Cmd::Config(_));
    let ctx = config_ctx::load(cli.config.as_deref(), cli.env.as_deref(), require_config)?;

    let cmd_name = command_name(&cli.cmd);
    let started = Instant::now();
    let result = match cli.cmd {
        Cmd::Introspect(args) => cmd_introspect::run(args, &ctx).await,
        Cmd::Query(args) => Box::pin(cmd_query::run(args, &ctx)).await,
        Cmd::Config(args) => cmd_config::run(args, &ctx),
        Cmd::Init(args) => cmd_init::run(args, &ctx).await,
        Cmd::Migrate(args) => cmd_migrate::run(args, &ctx).await,
        Cmd::Explain(args) => Box::pin(cmd_explain::run(args, &ctx)).await,
        Cmd::Diff(args) => Box::pin(cmd_diff::run(args, &ctx)).await,
        Cmd::Snapshot(args) => Box::pin(cmd_snapshot::run(args, &ctx)).await,
        Cmd::Seed(args) => Box::pin(cmd_seed::run(args, &ctx)).await,
        Cmd::Telemetry(args) => cmd_telemetry::run(&args),
    };
    let exit_code = i32::from(result.is_err());
    if let Some(rec) = telemetry::Recorder::from_env() {
        rec.record_command(cmd_name, exit_code, started.elapsed());
    }
    result
}

fn command_name(cmd: &Cmd) -> &'static str {
    match cmd {
        Cmd::Introspect(_) => "introspect",
        Cmd::Query(_) => "query",
        Cmd::Config(_) => "config",
        Cmd::Init(_) => "init",
        Cmd::Migrate(_) => "migrate",
        Cmd::Explain(_) => "explain",
        Cmd::Diff(_) => "diff",
        Cmd::Snapshot(_) => "snapshot",
        Cmd::Seed(_) => "seed",
        Cmd::Telemetry(_) => "telemetry",
    }
}
