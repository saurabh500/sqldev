//! `sqldev query` subcommand — one-shot T-SQL execution.
//!
//! M1.4 scope: text and JSON output for a single statement passed via
//! `--sql` or stdin. The REPL, table formatter, csv/ndjson, and stdin
//! pipeline live in M1.5.

use anyhow::{Context, Result, bail};
use clap::{Args as ClapArgs, ValueEnum};
use std::io::Read;

use crate::config_ctx::ConfigContext;
use crate::conn_flags::ConnectionFlags;

#[derive(ClapArgs, Debug)]
pub struct Args {
    #[command(flatten)]
    pub conn: ConnectionFlags,

    /// SQL to execute. If omitted, read from stdin.
    #[arg(long)]
    pub sql: Option<String>,

    /// Output format. `text` is human-readable; `json` is one JSON array of
    /// row objects (sufficient for piping through `jq`).
    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    pub format: OutputFormat,
}

#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum OutputFormat {
    Text,
    Json,
}

pub async fn run(args: Args, ctx: &ConfigContext) -> Result<()> {
    let sql = if let Some(s) = args.sql {
        s
    } else {
        let mut buf = String::new();
        std::io::stdin()
            .read_to_string(&mut buf)
            .context("read SQL from stdin")?;
        buf
    };
    if sql.trim().is_empty() {
        bail!("no SQL provided (use --sql or pipe via stdin)");
    }

    let opts = args
        .conn
        .resolve(ctx.env_block())
        .context("resolve connection options")?;
    let mut client = sqldev_conn::connect(&opts)
        .await
        .context("connect to SQL Server")?;
    let result_set = client
        .simple_query(sql)
        .await
        .context("execute query")?
        .into_first_result();

    match args.format {
        OutputFormat::Text => render_text(&result_set),
        OutputFormat::Json => render_json(&result_set)?,
    }
    Ok(())
}

fn render_text(rows: &[mssql_tiberius_bridge::Row]) {
    if rows.is_empty() {
        eprintln!("(no rows)");
        return;
    }
    // Header from the first row's column metadata.
    let cols: Vec<String> = rows[0]
        .columns()
        .iter()
        .map(|c| c.name().to_string())
        .collect();
    println!("{}", cols.join("\t"));
    for r in rows {
        let cells: Vec<String> = (0..cols.len()).map(|i| cell_to_string(r, i)).collect();
        println!("{}", cells.join("\t"));
    }
    eprintln!("({} row(s))", rows.len());
}

fn render_json(rows: &[mssql_tiberius_bridge::Row]) -> Result<()> {
    use serde_json::{Map, Value};
    let mut out: Vec<Value> = Vec::with_capacity(rows.len());
    for r in rows {
        let mut obj = Map::new();
        for (i, c) in r.columns().iter().enumerate() {
            obj.insert(c.name().to_string(), Value::String(cell_to_string(r, i)));
        }
        out.push(Value::Object(obj));
    }
    println!("{}", serde_json::to_string(&Value::Array(out))?);
    Ok(())
}

/// Render a single column value as a string. M1.4 trades type fidelity
/// for breadth — every value is a string. Typed JSON (numbers, bools,
/// null) lands when the formatter is rewritten in M1.5.
fn cell_to_string(row: &mssql_tiberius_bridge::Row, idx: usize) -> String {
    if let Some(v) = row.get::<&str, _>(idx) {
        return v.to_string();
    }
    if let Some(v) = row.get::<i64, _>(idx) {
        return v.to_string();
    }
    if let Some(v) = row.get::<i32, _>(idx) {
        return v.to_string();
    }
    if let Some(v) = row.get::<i16, _>(idx) {
        return v.to_string();
    }
    if let Some(v) = row.get::<bool, _>(idx) {
        return v.to_string();
    }
    if let Some(v) = row.get::<f64, _>(idx) {
        return v.to_string();
    }
    "NULL".to_string()
}
