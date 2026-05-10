//! `sqldev introspect` subcommand.

use anyhow::{Context, Result};
use clap::Args as ClapArgs;
use std::time::Instant;

use crate::config_ctx::ConfigContext;
use crate::conn_flags::ConnectionFlags;

#[derive(ClapArgs, Debug)]
pub struct Args {
    #[command(flatten)]
    pub conn: ConnectionFlags,

    /// Pretty-print the JSON output. Off by default for grep-friendliness.
    #[arg(long)]
    pub pretty: bool,
}

pub async fn run(args: Args, ctx: &ConfigContext) -> Result<()> {
    let opts = args
        .conn
        .resolve(ctx.env_block())
        .context("resolve connection options")?;
    let started = Instant::now();
    let mut client = sqldev_conn::connect(&opts)
        .await
        .context("connect to SQL Server")?;
    let graph = sqldev_introspect::build_schema_graph(&mut client, &opts.database)
        .await
        .context("build schema graph")?;

    if args.pretty {
        println!("{}", serde_json::to_string_pretty(&graph)?);
    } else {
        println!("{}", serde_json::to_string(&graph)?);
    }
    tracing::info!(
        database = opts.database,
        elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        "introspected"
    );
    Ok(())
}
