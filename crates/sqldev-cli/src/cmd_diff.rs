//! `sqldev diff` subcommand.
//!
//! Compares two schema graphs and emits a T-SQL migration script.
//! Each side may come from one of:
//!   * a path to a JSON schema-graph file (produced by `sqldev introspect`)
//!   * a live database (introspected on demand) via `--old-db` / `--new-db`

use anyhow::{Context, Result, bail};
use clap::Args as ClapArgs;
use std::fs;
use std::path::PathBuf;

use crate::config_ctx::ConfigContext;
use crate::conn_flags::ConnectionFlags;
use sqldev_core::SchemaGraph;

#[derive(ClapArgs, Debug)]
pub struct Args {
    /// Old (current) schema-graph JSON file.
    #[arg(long, conflicts_with = "old_db", group = "old_src")]
    pub old: Option<PathBuf>,
    /// New (target) schema-graph JSON file.
    #[arg(long, conflicts_with = "new_db", group = "new_src")]
    pub new: Option<PathBuf>,
    /// Treat the configured connection as the "old" side; live-introspect it.
    #[arg(long, conflicts_with = "old", group = "old_src")]
    pub old_db: bool,
    /// Treat the configured connection as the "new" side; live-introspect it.
    #[arg(long, conflicts_with = "new", group = "new_src")]
    pub new_db: bool,

    #[command(flatten)]
    pub conn: ConnectionFlags,

    /// Suppress the trailing `-- warnings` block on stderr.
    #[arg(long)]
    pub quiet: bool,
}

pub async fn run(args: Args, ctx: &ConfigContext) -> Result<()> {
    let old = load_side(args.old.as_deref(), args.old_db, &args.conn, ctx, "old").await?;
    let new = load_side(args.new.as_deref(), args.new_db, &args.conn, ctx, "new").await?;

    let result = sqldev_diff::diff(&old, &new);

    if result.statements.is_empty() {
        println!("-- no changes");
    } else {
        for stmt in &result.statements {
            println!("{stmt}");
        }
    }
    if !args.quiet {
        for w in &result.warnings {
            eprintln!("[diff] {w}");
        }
    }
    Ok(())
}

async fn load_side(
    file: Option<&std::path::Path>,
    live: bool,
    conn: &ConnectionFlags,
    ctx: &ConfigContext,
    side: &str,
) -> Result<SchemaGraph> {
    if let Some(path) = file {
        let text = fs::read_to_string(path)
            .with_context(|| format!("reading --{side} {}", path.display()))?;
        return serde_json::from_str(&text)
            .with_context(|| format!("parsing schema graph for --{side}"));
    }
    if live {
        let opts = conn
            .resolve(ctx.env_block())
            .context("resolve connection options")?;
        let mut client = sqldev_conn::connect(&opts)
            .await
            .context("connect to SQL Server")?;
        return sqldev_introspect::build_schema_graph(&mut client, &opts.database)
            .await
            .context("build schema graph");
    }
    bail!("must specify either --{side} <file> or --{side}-db");
}
