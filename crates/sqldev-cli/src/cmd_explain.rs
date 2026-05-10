//! `sqldev explain` — analyze a query plan for common anti-patterns.
//!
//! Three input modes:
//!
//! * `--sql "<T-SQL>"` — connect, set `SHOWPLAN_XML ON`, capture the
//!   plan (the query is **not** executed), then analyze.
//! * `--plan FILE` — read showplan XML from a local file.
//! * neither flag → read showplan XML from stdin.

use std::fs;
use std::io::{self, IsTerminal, Read, Write};
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use clap::{Args as ClapArgs, ValueEnum};

use crate::config_ctx::ConfigContext;
use crate::conn_flags::ConnectionFlags;

#[derive(ClapArgs, Debug)]
pub struct Args {
    #[command(flatten)]
    pub conn: ConnectionFlags,

    /// Run this T-SQL through `SET SHOWPLAN_XML ON` and analyze the
    /// returned plan. The statement is **not** executed.
    #[arg(long, conflicts_with = "plan")]
    pub sql: Option<String>,

    /// Read showplan XML from a local file.
    #[arg(long)]
    pub plan: Option<PathBuf>,

    /// Output format.
    #[arg(long, value_enum, default_value_t = ExplainFormat::Text)]
    pub format: ExplainFormat,
}

#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum ExplainFormat {
    /// Human-readable, one block per finding (default).
    Text,
    /// JSON array of findings.
    Json,
}

pub async fn run(args: Args, ctx: &ConfigContext) -> Result<()> {
    let xml = match (&args.sql, &args.plan) {
        (Some(sql), _) => fetch_plan(sql, &args.conn, ctx).await?,
        (None, Some(path)) => {
            fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?
        }
        (None, None) => {
            if io::stdin().is_terminal() {
                bail!(
                    "no input: pass `--sql \"<query>\"`, `--plan <file>`, or pipe showplan XML \
                     on stdin"
                );
            }
            let mut s = String::new();
            io::stdin().read_to_string(&mut s).context("read stdin")?;
            s
        }
    };

    let findings = sqldev_explain::analyze_plan(&xml).context("analyze showplan")?;

    let stdout = io::stdout();
    let mut out = stdout.lock();
    match args.format {
        ExplainFormat::Text => render_text(&findings, &mut out)?,
        ExplainFormat::Json => {
            serde_json::to_writer_pretty(&mut out, &findings).context("serialize findings")?;
            writeln!(out).ok();
        }
    }

    // Non-zero exit if any error-severity findings were emitted, so
    // CI scripts can fail builds on regressions.
    if findings
        .iter()
        .any(|f| matches!(f.severity, sqldev_explain::Severity::Error))
    {
        std::process::exit(1);
    }
    Ok(())
}

async fn fetch_plan(sql: &str, conn: &ConnectionFlags, ctx: &ConfigContext) -> Result<String> {
    let opts = conn
        .resolve(ctx.env_block())
        .context("resolve connection options")?;
    let mut client = sqldev_conn::connect(&opts)
        .await
        .context("connect to SQL Server")?;
    // Bracket the user's SQL between SHOWPLAN_XML ON / OFF so the
    // session is left clean even if the second batch errors.
    let batch = format!(
        "SET SHOWPLAN_XML ON;\n{};\nSET SHOWPLAN_XML OFF;\n",
        sql.trim_end_matches(';')
    );
    let rows = client
        .simple_query(batch)
        .await
        .context("execute SHOWPLAN_XML batch")?
        .into_first_result();
    if rows.is_empty() {
        bail!("SHOWPLAN_XML returned no rows; check that the server permits SHOWPLAN");
    }
    let mut xml = String::new();
    for row in &rows {
        if let Some(s) = row.try_get::<&str, _>(0).ok().flatten() {
            xml.push_str(s);
            xml.push('\n');
        }
    }
    if xml.trim().is_empty() {
        bail!("SHOWPLAN_XML returned no XML; the server may not have produced a plan");
    }
    Ok(xml)
}

fn render_text(findings: &[sqldev_explain::Finding], out: &mut impl Write) -> Result<()> {
    if findings.is_empty() {
        writeln!(out, "plan looks clean (no anti-patterns detected)")?;
        return Ok(());
    }
    for (i, f) in findings.iter().enumerate() {
        if i > 0 {
            writeln!(out)?;
        }
        writeln!(out, "[{}] {}", f.severity.as_str(), f.rule)?;
        if let Some(s) = &f.statement_text {
            writeln!(out, "    statement: {}", truncate(s, 100))?;
        }
        if let Some(t) = &f.target {
            writeln!(out, "    target:    {t}")?;
        }
        writeln!(out, "    finding:   {}", f.message)?;
        if let Some(c) = f.est_cost_delta {
            writeln!(out, "    est cost:  {c:.2}")?;
        }
    }
    writeln!(out)?;
    writeln!(out, "{} finding(s)", findings.len())?;
    Ok(())
}

fn truncate(s: &str, n: usize) -> String {
    let s = s.trim();
    if s.chars().count() <= n {
        s.to_string()
    } else {
        s.chars().take(n).collect::<String>() + "…"
    }
}
