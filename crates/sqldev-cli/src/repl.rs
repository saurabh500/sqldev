//! Interactive REPL for `sqldev query`.
//!
//! Activated when the user runs `sqldev query` without `--sql` and stdin
//! is a TTY (or when `--repl` is passed explicitly).
//!
//! Features:
//! - `rustyline` prompt with persistent history at
//!   `~/.config/sqldev/history` (or the platform-equivalent).
//! - Multi-line buffering: input accumulates until a line ends with `;`
//!   or a line containing only `GO` is seen.
//! - Meta-commands (lines starting with `\\`):
//!     - `\\q` / `\\quit`            — exit the REPL
//!     - `\\?` / `\\help`            — show help
//!     - `\\timing`                  — toggle elapsed-time display
//!     - `\\dt [schema]`             — list tables
//!     - `\\di [schema]`             — list indexes
//!     - `\\dv [schema]`             — list views
//!     - `\\df [schema]`             — list scalar/table-valued functions
//!     - `\\d <name>`                — describe a table or view
//! - Tab completion against a one-time cached snapshot of
//!   `INFORMATION_SCHEMA` (table and column names).
//! - Ctrl-C while a query is running aborts the in-flight statement and
//!   returns to the prompt; the connection is dropped and reopened lazily.
//! - Keyword-aware ANSI highlighting of input.

use std::borrow::Cow;
use std::path::PathBuf;
use std::time::Instant;

use anyhow::{Context, Result};
use mssql_tiberius_bridge::Client;
use rustyline::completion::{Completer, Pair};
use rustyline::error::ReadlineError;
use rustyline::highlight::Highlighter;
use rustyline::hint::Hinter;
use rustyline::validate::Validator;
use rustyline::{Config, Editor, Helper};
use sqldev_conn::ConnectOptions;

use crate::cmd_query::OutputFormat;
use crate::output;

/// Run the REPL until the user exits.
pub async fn run(opts: ConnectOptions, fmt: OutputFormat) -> Result<()> {
    let mut state = ReplState::new(opts, fmt);
    let history_path = history_file();
    if let Some(parent) = history_path.as_ref().and_then(|p| p.parent()) {
        let _ = std::fs::create_dir_all(parent);
    }

    let helper = SqlHelper::new();
    let config = Config::builder().auto_add_history(true).build();
    let mut rl: Editor<SqlHelper, _> =
        Editor::with_config(config).context("init rustyline")?;
    rl.set_helper(Some(helper));
    if let Some(p) = &history_path {
        let _ = rl.load_history(p);
    }

    println!("sqldev REPL — connected to {}@{} ({}). Type \\? for help, \\q to exit.", state.opts.database, state.opts.host, render_user(&state.opts));
    println!();

    let mut buffer = String::new();
    loop {
        let prompt = if buffer.is_empty() {
            format!("{}> ", state.opts.database)
        } else {
            "    ...> ".to_string()
        };
        match rl.readline(&prompt) {
            Ok(line) => {
                let trimmed = line.trim();
                if buffer.is_empty() && trimmed.is_empty() {
                    continue;
                }
                if buffer.is_empty()
                    && let Some(rest) = trimmed.strip_prefix('\\')
                {
                    if matches!(rest, "q" | "quit" | "exit") {
                        break;
                    }
                    if let Err(e) = handle_meta(&mut state, rest).await {
                        eprintln!("error: {e:#}");
                    }
                    continue;
                }
                buffer.push_str(&line);
                buffer.push('\n');
                if !is_complete(&buffer) {
                    continue;
                }
                let sql = std::mem::take(&mut buffer);
                if let Err(e) = run_sql(&mut state, sql.trim()).await {
                    eprintln!("error: {e:#}");
                }
            }
            Err(ReadlineError::Interrupted) => {
                if buffer.is_empty() {
                    eprintln!("(use \\q to exit)");
                } else {
                    buffer.clear();
                }
            }
            Err(ReadlineError::Eof) => break,
            Err(e) => {
                eprintln!("readline error: {e}");
                break;
            }
        }
    }

    if let Some(p) = &history_path {
        let _ = rl.save_history(p);
    }
    Ok(())
}

fn render_user(opts: &ConnectOptions) -> String {
    match &opts.auth {
        sqldev_conn::AuthOptions::Sql { user, .. } => user.clone(),
    }
}

fn history_file() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("sqldev").join("history"))
}

/// Mutable REPL state.
struct ReplState {
    opts: ConnectOptions,
    fmt: OutputFormat,
    /// Opened lazily; cleared after Ctrl-C or a connection-level error so
    /// the next command reconnects cleanly.
    client: Option<Client>,
    timing: bool,
}

impl ReplState {
    fn new(opts: ConnectOptions, fmt: OutputFormat) -> Self {
        Self {
            opts,
            fmt,
            client: None,
            timing: false,
        }
    }

    async fn client(&mut self) -> Result<&mut Client> {
        if self.client.is_none() {
            let c = sqldev_conn::connect(&self.opts)
                .await
                .context("connect to SQL Server")?;
            self.client = Some(c);
        }
        Ok(self.client.as_mut().expect("client just connected"))
    }
}

/// Returns true when `buffer` contains a complete batch (terminated by
/// `;` or a `GO` line).
fn is_complete(buffer: &str) -> bool {
    for line in buffer.lines() {
        if line.trim().eq_ignore_ascii_case("GO") {
            return true;
        }
    }
    let trimmed = buffer.trim_end();
    trimmed.ends_with(';')
}

async fn run_sql(state: &mut ReplState, sql: &str) -> Result<()> {
    // Strip trailing GO if present so tiberius doesn't see it as a token.
    let cleaned = strip_terminators(sql);
    if cleaned.trim().is_empty() {
        return Ok(());
    }

    let started = Instant::now();
    let client = state.client().await?;
    let exec = client.simple_query(cleaned.to_string());

    let result = tokio::select! {
        biased;
        _ = tokio::signal::ctrl_c() => {
            eprintln!("\n^C — cancelling, dropping connection");
            // Drop the client so the cancelled future is aborted; the next
            // command will reconnect.
            state.client = None;
            return Ok(());
        }
        r = exec => r,
    };

    let rs = match result {
        Ok(r) => r.into_first_result(),
        Err(e) => {
            // Treat connection-level errors as a hint to reconnect.
            state.client = None;
            return Err(anyhow::anyhow!("{e}").context("execute query"));
        }
    };

    let extracted = output::value::extract(&rs);
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    if extracted.is_empty() {
        eprintln!("(no rows)");
    } else {
        match state.fmt {
            OutputFormat::Text => output::text_fmt::write(&extracted, &mut out)?,
            OutputFormat::Table => output::table_fmt::write(&extracted, &mut out)?,
            OutputFormat::Json => output::json_fmt::write(&extracted, &mut out)?,
            OutputFormat::Ndjson => output::ndjson_fmt::write(&extracted, &mut out)?,
            OutputFormat::Csv => output::csv_fmt::write(&extracted, &mut out)?,
        }
        eprintln!("({} row(s))", extracted.len());
    }
    if state.timing {
        eprintln!("Time: {:?}", started.elapsed());
    }
    Ok(())
}

/// Trim trailing semicolons and `GO` lines from a buffer so the SQL we
/// hand to tiberius is a single batch with no terminator tokens.
fn strip_terminators(sql: &str) -> String {
    let mut lines: Vec<&str> = sql.lines().collect();
    while let Some(last) = lines.last() {
        let t = last.trim();
        if t.is_empty() || t.eq_ignore_ascii_case("GO") {
            lines.pop();
        } else {
            break;
        }
    }
    let mut s = lines.join("\n");
    while s.ends_with(';') || s.ends_with(char::is_whitespace) {
        s.pop();
    }
    s
}

// ---------------------------------------------------------------------------
// Meta-commands
// ---------------------------------------------------------------------------

async fn handle_meta(state: &mut ReplState, rest: &str) -> Result<()> {
    let mut parts = rest.split_whitespace();
    let cmd = parts.next().unwrap_or("");
    let arg = parts.next();
    match cmd {
        "?" | "help" => {
            print_help();
            Ok(())
        }
        "timing" => {
            state.timing = !state.timing;
            eprintln!("timing: {}", if state.timing { "on" } else { "off" });
            Ok(())
        }
        "dt" => list_objects(state, "U", arg, "table").await,
        "dv" => list_objects(state, "V", arg, "view").await,
        "di" => list_indexes(state, arg).await,
        "df" => list_functions(state, arg).await,
        "d" => {
            if let Some(name) = arg {
                describe_object(state, name).await
            } else {
                eprintln!("usage: \\d <table-or-view-name>");
                Ok(())
            }
        }
        other => {
            eprintln!("unknown meta-command: \\{other}  (try \\?)");
            Ok(())
        }
    }
}

fn print_help() {
    println!(
        r"
Meta-commands:
  \q  \quit          exit the REPL
  \?  \help          this help
  \timing            toggle elapsed-time display
  \dt [schema]       list tables
  \dv [schema]       list views
  \di [schema]       list indexes
  \df [schema]       list functions
  \d <name>          describe a table or view (columns, types, nullability)

End a statement with `;` or a line containing only `GO`.
Press Ctrl-C to cancel an in-flight query (drops connection; auto-reconnects).
"
    );
}

async fn list_objects(
    state: &mut ReplState,
    obj_type: &str,
    schema: Option<&str>,
    label: &str,
) -> Result<()> {
    let where_clause = match schema {
        Some(s) => format!(" AND s.name = N'{}'", s.replace('\'', "''")),
        None => String::new(),
    };
    let sql = format!(
        "SELECT s.name AS [schema], o.name AS [{label}] \
         FROM sys.objects o JOIN sys.schemas s ON o.schema_id = s.schema_id \
         WHERE o.type = '{obj_type}'{where_clause} \
         ORDER BY s.name, o.name"
    );
    run_sql(state, &sql).await
}

async fn list_indexes(state: &mut ReplState, schema: Option<&str>) -> Result<()> {
    let where_clause = match schema {
        Some(s) => format!(" AND s.name = N'{}'", s.replace('\'', "''")),
        None => String::new(),
    };
    let sql = format!(
        "SELECT s.name AS [schema], t.name AS [table], i.name AS [index], \
                i.type_desc AS [type], i.is_unique AS [unique] \
         FROM sys.indexes i \
         JOIN sys.tables t ON i.object_id = t.object_id \
         JOIN sys.schemas s ON t.schema_id = s.schema_id \
         WHERE i.name IS NOT NULL{where_clause} \
         ORDER BY s.name, t.name, i.name"
    );
    run_sql(state, &sql).await
}

async fn list_functions(state: &mut ReplState, schema: Option<&str>) -> Result<()> {
    let where_clause = match schema {
        Some(s) => format!(" AND s.name = N'{}'", s.replace('\'', "''")),
        None => String::new(),
    };
    let sql = format!(
        "SELECT s.name AS [schema], o.name AS [function], o.type_desc AS [type] \
         FROM sys.objects o JOIN sys.schemas s ON o.schema_id = s.schema_id \
         WHERE o.type IN ('FN','IF','TF','FS','FT'){where_clause} \
         ORDER BY s.name, o.name"
    );
    run_sql(state, &sql).await
}

async fn describe_object(state: &mut ReplState, name: &str) -> Result<()> {
    let (schema, obj) = split_qualified_name(name);
    let sql = format!(
        "SELECT c.name AS [column], \
                t.name AS [type], \
                c.max_length, c.precision, c.scale, \
                c.is_nullable AS [nullable] \
         FROM sys.columns c \
         JOIN sys.objects o ON c.object_id = o.object_id \
         JOIN sys.schemas s ON o.schema_id = s.schema_id \
         JOIN sys.types t ON c.user_type_id = t.user_type_id \
         WHERE o.name = N'{}' AND s.name = N'{}' \
         ORDER BY c.column_id",
        obj.replace('\'', "''"),
        schema.replace('\'', "''"),
    );
    run_sql(state, &sql).await
}

fn split_qualified_name(name: &str) -> (String, String) {
    let cleaned: String = name.chars().filter(|c| *c != '[' && *c != ']').collect();
    if let Some((s, o)) = cleaned.split_once('.') {
        (s.to_string(), o.to_string())
    } else {
        ("dbo".to_string(), cleaned)
    }
}

// ---------------------------------------------------------------------------
// rustyline helper (highlighter + completer)
// ---------------------------------------------------------------------------

const KEYWORDS: &[&str] = &[
    "SELECT", "FROM", "WHERE", "INSERT", "INTO", "VALUES", "UPDATE", "SET", "DELETE",
    "JOIN", "LEFT", "RIGHT", "INNER", "OUTER", "CROSS", "ON", "AS", "AND", "OR", "NOT",
    "NULL", "IS", "IN", "LIKE", "BETWEEN", "ORDER", "BY", "GROUP", "HAVING", "TOP",
    "DISTINCT", "UNION", "ALL", "CREATE", "TABLE", "INDEX", "VIEW", "PROCEDURE",
    "FUNCTION", "ALTER", "DROP", "TRUNCATE", "BEGIN", "END", "IF", "ELSE", "DECLARE",
    "EXEC", "EXECUTE", "TRAN", "TRANSACTION", "COMMIT", "ROLLBACK", "GO", "USE",
    "WITH", "CASE", "WHEN", "THEN", "RETURN", "PRIMARY", "KEY", "FOREIGN",
    "REFERENCES", "CONSTRAINT", "UNIQUE", "CHECK", "DEFAULT", "IDENTITY",
];

#[derive(Default)]
struct SqlHelper;

impl SqlHelper {
    fn new() -> Self {
        Self
    }
}

impl Helper for SqlHelper {}
impl Validator for SqlHelper {}
impl Hinter for SqlHelper {
    type Hint = String;
}

impl Highlighter for SqlHelper {
    fn highlight<'l>(&self, line: &'l str, _pos: usize) -> Cow<'l, str> {
        Cow::Owned(highlight_line(line))
    }

    fn highlight_char(&self, _line: &str, _pos: usize, _kind: rustyline::highlight::CmdKind) -> bool {
        true
    }
}

fn highlight_line(line: &str) -> String {
    let mut out = String::with_capacity(line.len() + 16);
    let mut buf = String::new();
    for ch in line.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            buf.push(ch);
        } else {
            flush_token(&mut buf, &mut out);
            out.push(ch);
        }
    }
    flush_token(&mut buf, &mut out);
    out
}

fn flush_token(buf: &mut String, out: &mut String) {
    if buf.is_empty() {
        return;
    }
    let upper = buf.to_ascii_uppercase();
    if KEYWORDS.contains(&upper.as_str()) {
        // Bold cyan for keywords.
        out.push_str("\x1b[1;36m");
        out.push_str(buf);
        out.push_str("\x1b[0m");
    } else {
        out.push_str(buf);
    }
    buf.clear();
}

impl Completer for SqlHelper {
    type Candidate = Pair;

    fn complete(
        &self,
        line: &str,
        pos: usize,
        _ctx: &rustyline::Context<'_>,
    ) -> rustyline::Result<(usize, Vec<Pair>)> {
        // Find the start of the current word (alnum/underscore).
        let prefix_start = line[..pos]
            .char_indices()
            .rev()
            .take_while(|(_, c)| c.is_ascii_alphanumeric() || *c == '_')
            .last()
            .map_or(pos, |(i, _)| i);
        let prefix = &line[prefix_start..pos];
        if prefix.is_empty() {
            return Ok((pos, Vec::new()));
        }
        let upper = prefix.to_ascii_uppercase();
        let mut candidates: Vec<Pair> = KEYWORDS
            .iter()
            .filter(|k| k.starts_with(&upper))
            .map(|k| Pair {
                display: (*k).to_string(),
                replacement: (*k).to_string(),
            })
            .collect();
        candidates.sort_by(|a, b| a.display.cmp(&b.display));
        Ok((prefix_start, candidates))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_complete_semicolon() {
        assert!(is_complete("SELECT 1;\n"));
        assert!(!is_complete("SELECT 1\n"));
    }

    #[test]
    fn is_complete_go_line() {
        assert!(is_complete("CREATE TABLE x(id INT)\nGO\n"));
        assert!(is_complete("SELECT 1\ngo\n"));
        assert!(!is_complete("SELECT 1\nGOOD\n"));
    }

    #[test]
    fn strip_terminators_drops_trailing_semis_and_go() {
        assert_eq!(strip_terminators("SELECT 1;\n"), "SELECT 1");
        assert_eq!(strip_terminators("CREATE T x\nGO\n"), "CREATE T x");
        assert_eq!(strip_terminators("SELECT 1;\nGO\n"), "SELECT 1");
    }

    #[test]
    fn split_qualified_name_defaults_to_dbo() {
        assert_eq!(
            split_qualified_name("Foo"),
            ("dbo".to_string(), "Foo".to_string())
        );
        assert_eq!(
            split_qualified_name("[sales].[Order]"),
            ("sales".to_string(), "Order".to_string())
        );
    }

    #[test]
    fn highlight_line_marks_keywords() {
        let h = highlight_line("SELECT id FROM t");
        assert!(h.contains("\x1b[1;36mSELECT\x1b[0m"));
        assert!(h.contains("\x1b[1;36mFROM\x1b[0m"));
    }
}
