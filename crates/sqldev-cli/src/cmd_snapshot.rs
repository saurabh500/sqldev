//! `sqldev snapshot` subcommand.
//!
//! Captures the live schema graph as JSON for offline diffing in CI.
//!
//! ```bash
//! sqldev snapshot save                      # → stdout
//! sqldev snapshot save dev                  # → snapshots/dev.json
//! sqldev snapshot save --output graph.json  # → graph.json
//! ```

use anyhow::{Context, Result, bail};
use clap::{Args as ClapArgs, Subcommand};
use std::fs;
use std::path::PathBuf;

use crate::config_ctx::ConfigContext;
use crate::conn_flags::ConnectionFlags;

#[derive(ClapArgs, Debug)]
pub struct Args {
    #[command(subcommand)]
    pub cmd: Sub,
}

#[derive(Subcommand, Debug)]
pub enum Sub {
    /// Introspect the configured database and write its schema graph as JSON.
    Save(SaveArgs),
}

#[derive(ClapArgs, Debug)]
pub struct SaveArgs {
    /// Snapshot name. When set (and `--output` is not), the file is written
    /// to `<dir>/<name>.json` where `<dir>` is `--dir` (default
    /// `snapshots`). When omitted, the graph is written to stdout unless
    /// `--output` is given.
    pub name: Option<String>,

    /// Explicit output file. Mutually exclusive with `name`.
    #[arg(long, conflicts_with = "name")]
    pub output: Option<PathBuf>,

    /// Directory used when only a snapshot `name` is given.
    #[arg(long, default_value = "snapshots")]
    pub dir: PathBuf,

    /// Pretty-print the JSON output.
    #[arg(long)]
    pub pretty: bool,

    /// Overwrite an existing snapshot file (otherwise the command fails).
    #[arg(long)]
    pub force: bool,

    #[command(flatten)]
    pub conn: ConnectionFlags,
}

pub async fn run(args: Args, ctx: &ConfigContext) -> Result<()> {
    match args.cmd {
        Sub::Save(a) => run_save(a, ctx).await,
    }
}

async fn run_save(args: SaveArgs, ctx: &ConfigContext) -> Result<()> {
    let opts = args
        .conn
        .resolve(ctx.env_block())
        .context("resolve connection options")?;
    let mut client = sqldev_conn::connect(&opts)
        .await
        .context("connect to SQL Server")?;
    let graph = sqldev_introspect::build_schema_graph(&mut client, &opts.database)
        .await
        .context("build schema graph")?;

    let json = if args.pretty {
        serde_json::to_string_pretty(&graph)?
    } else {
        serde_json::to_string(&graph)?
    };

    let path = match (&args.output, &args.name) {
        (Some(p), _) => Some(p.clone()),
        (None, Some(name)) => Some(args.dir.join(format!("{}.json", sanitize_name(name)))),
        (None, None) => None,
    };

    if let Some(path) = path {
        if path.exists() && !args.force {
            bail!(
                "{} already exists (pass --force to overwrite)",
                path.display()
            );
        }
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
        }
        fs::write(&path, &json).with_context(|| format!("write {}", path.display()))?;
        println!("Wrote {}", path.display());
    } else {
        println!("{json}");
    }
    Ok(())
}

fn sanitize_name(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
            out.push(ch);
        } else if ch == ' ' {
            out.push('_');
        }
    }
    if out.is_empty() {
        out.push_str("snapshot");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::sanitize_name;

    #[test]
    fn sanitize_keeps_alphanumeric_dash_underscore_dot() {
        assert_eq!(sanitize_name("dev-1.2_x"), "dev-1.2_x");
    }

    #[test]
    fn sanitize_replaces_spaces_and_strips_others() {
        assert_eq!(sanitize_name("my snap/!"), "my_snap");
    }

    #[test]
    fn sanitize_falls_back_to_default() {
        assert_eq!(sanitize_name("///"), "snapshot");
    }
}
