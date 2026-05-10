//! `sqldev query` subcommand — one-shot T-SQL execution.
//!
//! M1.4 polish: typed values flow through the [`crate::output`] layer
//! and are rendered as one of `text` (TSV), `table` (aligned),
//! `json` (typed array), `ndjson`, or `csv`.

use anyhow::{Context, Result, bail};
use clap::{Args as ClapArgs, ValueEnum};
use std::io::{self, Read};

use crate::config_ctx::ConfigContext;
use crate::conn_flags::ConnectionFlags;
use crate::output;

#[derive(ClapArgs, Debug)]
pub struct Args {
    #[command(flatten)]
    pub conn: ConnectionFlags,

    /// SQL to execute. If omitted, read from stdin.
    #[arg(long)]
    pub sql: Option<String>,

    /// Output format. `text` is tab-separated and pipe-friendly;
    /// `table` is an aligned, human-readable table; `json` is a typed
    /// JSON array; `ndjson` is newline-delimited JSON; `csv` is
    /// RFC 4180.
    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    pub format: OutputFormat,
}

#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum OutputFormat {
    Text,
    Table,
    Json,
    Ndjson,
    Csv,
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

    let rs = output::value::extract(&result_set);
    if rs.is_empty() {
        eprintln!("(no rows)");
        return Ok(());
    }

    let stdout = io::stdout();
    let mut out = stdout.lock();
    match args.format {
        OutputFormat::Text => output::text_fmt::write(&rs, &mut out)?,
        OutputFormat::Table => output::table_fmt::write(&rs, &mut out)?,
        OutputFormat::Json => output::json_fmt::write(&rs, &mut out)?,
        OutputFormat::Ndjson => output::ndjson_fmt::write(&rs, &mut out)?,
        OutputFormat::Csv => output::csv_fmt::write(&rs, &mut out)?,
    }
    eprintln!("({} row(s))", rs.len());
    Ok(())
}
