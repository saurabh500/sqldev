//! `sqldev seed` subcommand.
//!
//! Drives [`sqldev_seed`]: loads YAML seed files, introspects the live
//! schema for type info, generates `INSERT` batches, and either prints
//! them (`--dry-run`) or executes them inside a single transaction.

use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result, anyhow, bail};
use clap::Args as ClapArgs;
use sqldev_seed::{Generator, InsertBatch, SeedFile};

use crate::config_ctx::ConfigContext;
use crate::conn_flags::ConnectionFlags;

#[derive(ClapArgs, Debug)]
pub struct Args {
    /// Single YAML seed file. Mutually exclusive with `--dir`.
    #[arg(long, conflicts_with = "dir")]
    pub file: Option<PathBuf>,

    /// Directory of `*.yml` / `*.yaml` seed files (default `seeds`).
    #[arg(long)]
    pub dir: Option<PathBuf>,

    /// Only seed tables whose `seed.table` matches `<schema>.<table>` or
    /// just `<table>`. Repeatable.
    #[arg(long = "table")]
    pub tables: Vec<String>,

    /// Deterministic mode: identical `--seed N` runs produce
    /// byte-for-byte identical SQL.
    #[arg(long = "seed")]
    pub seed: Option<u64>,

    /// Print the generated `INSERT` SQL instead of executing it.
    #[arg(long)]
    pub dry_run: bool,

    #[command(flatten)]
    pub conn: ConnectionFlags,
}

pub async fn run(args: Args, ctx: &ConfigContext) -> Result<()> {
    let seed_files = resolve_seed_files(&args)?;
    if seed_files.is_empty() {
        println!("-- no seed files found");
        return Ok(());
    }

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

    let mut g = match args.seed {
        Some(n) => Generator::from_seed(n),
        None => Generator::from_entropy(),
    };

    // Build all batches up front so a generation error in file 3 doesn't
    // leave files 1-2 half-applied.
    let mut planned: Vec<(PathBuf, String, Vec<InsertBatch>)> = Vec::new();
    for (path, seed) in &seed_files {
        if !table_matches(&args.tables, &seed.table) {
            continue;
        }
        let table = sqldev_seed::find_table(&graph, &seed.table).ok_or_else(|| {
            anyhow!(
                "{}: table `{}` not found in database `{}`",
                path.display(),
                seed.table,
                opts.database,
            )
        })?;
        let batches = sqldev_seed::build_inserts(seed, table, path, &mut g)
            .with_context(|| format!("generate seed rows for {}", path.display()))?;
        planned.push((path.clone(), seed.table.clone(), batches));
    }

    if args.dry_run {
        for (path, table, batches) in &planned {
            println!("-- {} → {}", path.display(), table);
            for batch in batches {
                println!("{}", batch.sql);
            }
            println!();
        }
        return Ok(());
    }

    // Wrap the entire seed run in a transaction so partial failures roll
    // back cleanly. Mirrors `cmd_migrate`.
    client
        .simple_query("BEGIN TRAN sqldev_seed;")
        .await
        .context("begin transaction")?;
    let result: Result<()> = async {
        for (path, table, batches) in &planned {
            let mut rows = 0usize;
            for batch in batches {
                client
                    .simple_query(batch.sql.clone())
                    .await
                    .with_context(|| format!("execute insert batch into {table}"))?;
                rows += batch.row_count;
            }
            tracing::info!(
                file = %path.display(),
                table = %table,
                rows,
                "seeded",
            );
            println!("Inserted {rows} row(s) into {table} ({})", path.display());
        }
        Ok(())
    }
    .await;

    if let Err(e) = result {
        let _ = client
            .simple_query("IF @@TRANCOUNT > 0 ROLLBACK TRAN;")
            .await;
        return Err(e);
    }
    client
        .simple_query("COMMIT TRAN sqldev_seed;")
        .await
        .context("commit transaction")?;
    Ok(())
}

fn resolve_seed_files(args: &Args) -> Result<Vec<(PathBuf, SeedFile)>> {
    if let Some(file) = &args.file {
        let seed = sqldev_seed::load_seed_file(file)
            .with_context(|| format!("load seed file {}", file.display()))?;
        return Ok(vec![(file.clone(), seed)]);
    }
    let dir = args.dir.clone().unwrap_or_else(|| PathBuf::from("seeds"));
    if !dir.exists() {
        if args.dir.is_some() {
            bail!("seed dir {} does not exist", dir.display());
        }
        return Ok(vec![]);
    }
    let metadata = fs::metadata(&dir).with_context(|| format!("stat {}", dir.display()))?;
    if !metadata.is_dir() {
        bail!("{} is not a directory", dir.display());
    }
    let loaded = sqldev_seed::load_seed_dir(&dir)
        .with_context(|| format!("read seed dir {}", dir.display()))?;
    Ok(loaded)
}

fn table_matches(filters: &[String], table: &str) -> bool {
    if filters.is_empty() {
        return true;
    }
    let (s, t) = sqldev_seed::split_qualified(table);
    let qualified = format!("{s}.{t}");
    filters
        .iter()
        .any(|f| f == table || f == &qualified || f == &t)
}

#[cfg(test)]
mod tests {
    use super::table_matches;

    #[test]
    fn matches_when_no_filter() {
        assert!(table_matches(&[], "dbo.Customer"));
    }

    #[test]
    fn matches_qualified_and_bare() {
        let f = vec!["Customer".to_string()];
        assert!(table_matches(&f, "dbo.Customer"));
        assert!(table_matches(&f, "Customer"));
        assert!(!table_matches(&f, "dbo.Order"));
    }

    #[test]
    fn matches_full_qualified() {
        let f = vec!["sales.Order".to_string()];
        assert!(table_matches(&f, "sales.Order"));
        assert!(!table_matches(&f, "dbo.Order"));
    }
}
