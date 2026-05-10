//! `sqldev migrate` — apply versioned T-SQL migrations.
//!
//! ## File format
//!
//! Each migration lives in `migrations/NNNN_<name>.sql`, where `NNNN` is a
//! numeric version (zero-padded by convention). The file may contain two
//! sections delimited by directive comments:
//!
//! ```sql
//! -- +sqldev Up
//! CREATE TABLE ...
//! -- +sqldev Down
//! DROP TABLE ...
//! ```
//!
//! When no markers are present the entire file is treated as the `Up`
//! section and there is no `Down`.
//!
//! Within each section, batches are separated by a line containing only
//! `GO` (case-insensitive, optional surrounding whitespace), matching
//! `sqlcmd` semantics.
//!
//! ## Tracking
//!
//! On first use the CLI creates `dbo.__sqldev_migrations`:
//!
//! ```sql
//! CREATE TABLE dbo.__sqldev_migrations (
//!     version       NVARCHAR(255) NOT NULL CONSTRAINT PK_sqldev_migrations PRIMARY KEY,
//!     name          NVARCHAR(500) NOT NULL,
//!     applied_at    DATETIME2(3)  NOT NULL CONSTRAINT DF_sqldev_migrations_applied_at DEFAULT SYSUTCDATETIME(),
//!     checksum      NVARCHAR(64)  NOT NULL
//! );
//! ```
//!
//! ## Concurrency
//!
//! Before any migration runs, the session takes `sp_getapplock` on
//! resource `sqldev:migrate` (Exclusive, Session). A second concurrent
//! migrator blocks until the first completes (or the 30s timeout fires,
//! at which point the second exits with an error).

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Args as ClapArgs, Subcommand};
use mssql_tiberius_bridge::Client;
use sha2::{Digest, Sha256};

use crate::config_ctx::ConfigContext;
use crate::conn_flags::ConnectionFlags;

#[derive(ClapArgs, Debug)]
pub struct Args {
    #[command(subcommand)]
    cmd: Sub,
}

#[derive(Subcommand, Debug)]
enum Sub {
    /// Create a new empty migration file with up/down sections.
    Create(CreateArgs),
    /// Show applied vs pending migrations.
    Status(StatusArgs),
    /// Apply pending migrations.
    Up(ApplyArgs),
    /// Roll back applied migrations.
    Down(ApplyArgs),
}

#[derive(ClapArgs, Debug)]
struct CreateArgs {
    /// Short `snake_case` name for the migration.
    name: String,

    /// Project root containing `migrations/`. Defaults to the current dir.
    #[arg(long)]
    path: Option<PathBuf>,
}

#[derive(ClapArgs, Debug)]
struct StatusArgs {
    #[command(flatten)]
    conn: ConnectionFlags,

    /// Project root containing `migrations/`. Defaults to the current dir.
    #[arg(long)]
    path: Option<PathBuf>,
}

#[derive(ClapArgs, Debug)]
struct ApplyArgs {
    #[command(flatten)]
    conn: ConnectionFlags,

    /// Project root containing `migrations/`. Defaults to the current dir.
    #[arg(long)]
    path: Option<PathBuf>,

    /// Apply/roll back through this version (inclusive).
    #[arg(long)]
    to: Option<String>,

    /// Apply/roll back at most N migrations.
    #[arg(long)]
    steps: Option<usize>,

    /// Print the SQL that would run, but make no changes.
    #[arg(long)]
    dry_run: bool,

    /// Required when the resolved env block is `protected: true`.
    /// Value must equal the env block name as a guard against typos.
    #[arg(long)]
    confirm: Option<String>,
}

pub async fn run(args: Args, ctx: &ConfigContext) -> Result<()> {
    match args.cmd {
        Sub::Create(a) => run_create(a),
        Sub::Status(a) => run_status(a, ctx).await,
        Sub::Up(a) => run_apply(a, ctx, Direction::Up).await,
        Sub::Down(a) => run_apply(a, ctx, Direction::Down).await,
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Direction {
    Up,
    Down,
}

// ---------------------------------------------------------------------------
// migrate create
// ---------------------------------------------------------------------------

fn run_create(args: CreateArgs) -> Result<()> {
    let root = args.path.unwrap_or_else(|| PathBuf::from("."));
    let dir = root.join("migrations");
    fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;

    let next = next_version(&dir)?;
    let file_name = format!("{:04}_{}.sql", next, sanitize_name(&args.name));
    let path = dir.join(&file_name);
    if path.exists() {
        bail!("{} already exists", path.display());
    }

    let body = format!(
        "-- {file_name}\n\
         -- Created by `sqldev migrate create`.\n\n\
         -- +sqldev Up\n\n\n\
         -- +sqldev Down\n\n"
    );
    fs::write(&path, body).with_context(|| format!("write {}", path.display()))?;
    println!("Created {}", path.display());
    Ok(())
}

fn sanitize_name(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if matches!(ch, ' ' | '-' | '_') {
            out.push('_');
        }
    }
    if out.is_empty() {
        out.push_str("migration");
    }
    out
}

fn next_version(dir: &Path) -> Result<u32> {
    let mut max_seen: u32 = 0;
    if !dir.exists() {
        return Ok(1);
    }
    for entry in fs::read_dir(dir).with_context(|| format!("read {}", dir.display()))? {
        let entry = entry?;
        if let Some(file) = MigrationFile::parse_name(&entry.file_name().to_string_lossy())
            && let Ok(n) = file.version.parse::<u32>()
        {
            max_seen = max_seen.max(n);
        }
    }
    Ok(max_seen + 1)
}

// ---------------------------------------------------------------------------
// migrate status
// ---------------------------------------------------------------------------

async fn run_status(args: StatusArgs, ctx: &ConfigContext) -> Result<()> {
    let root = args.path.unwrap_or_else(|| PathBuf::from("."));
    let local = load_local(&root)?;

    let opts = args
        .conn
        .resolve(ctx.env_block())
        .context("resolve connection options")?;
    let mut client = sqldev_conn::connect(&opts)
        .await
        .context("connect to SQL Server")?;
    ensure_tracking_table(&mut client).await?;
    let applied = load_applied(&mut client).await?;

    let mut versions: Vec<&str> = local.keys().map(String::as_str).collect();
    for v in applied.keys() {
        if !local.contains_key(v) {
            versions.push(v);
        }
    }
    versions.sort_unstable();
    versions.dedup();

    println!("VERSION      STATUS     APPLIED_AT               NAME");
    for v in versions {
        let (status, applied_at, name) = match (local.get(v), applied.get(v)) {
            (Some(file), Some(rec)) => {
                let label = if rec.checksum == file.checksum {
                    "applied"
                } else {
                    "drifted"
                };
                (label, rec.applied_at.clone(), file.name.clone())
            }
            (Some(file), None) => ("pending", String::from("-"), file.name.clone()),
            (None, Some(rec)) => ("missing", rec.applied_at.clone(), rec.name.clone()),
            (None, None) => unreachable!(),
        };
        println!("{v:<12} {status:<10} {applied_at:<24} {name}");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// migrate up / down
// ---------------------------------------------------------------------------

async fn run_apply(args: ApplyArgs, ctx: &ConfigContext, dir: Direction) -> Result<()> {
    // Protected-env guard.
    if let Some((name, env)) = ctx.env_block()
        && env.protected
    {
        match &args.confirm {
            Some(c) if c == name => {}
            _ => bail!(
                "env `{name}` is protected; pass --confirm {name} to apply migrations against it"
            ),
        }
    }

    let root = args.path.clone().unwrap_or_else(|| PathBuf::from("."));
    let local = load_local(&root)?;
    if local.is_empty() {
        eprintln!(
            "(no migrations found in {})",
            root.join("migrations").display()
        );
        return Ok(());
    }

    let opts = args
        .conn
        .resolve(ctx.env_block())
        .context("resolve connection options")?;
    let mut client = sqldev_conn::connect(&opts)
        .await
        .context("connect to SQL Server")?;

    if !args.dry_run {
        acquire_app_lock(&mut client).await?;
    }
    ensure_tracking_table(&mut client).await?;
    let applied = load_applied(&mut client).await?;

    let plan = plan(&local, &applied, dir, args.to.as_deref(), args.steps)?;
    if plan.is_empty() {
        eprintln!("Nothing to do.");
        return Ok(());
    }

    for step in &plan {
        let file = local
            .get(&step.version)
            .expect("planned version came from local map");
        let sql = match dir {
            Direction::Up => &file.up_sql,
            Direction::Down => &file.down_sql,
        };
        let sql = sql.as_deref().unwrap_or("").trim();
        if sql.is_empty() && dir == Direction::Down {
            bail!(
                "migration {} ({}) has no `-- +sqldev Down` section; cannot roll back",
                file.version,
                file.name
            );
        }

        if args.dry_run {
            println!("-- {} {} ({})", arrow(dir), file.version, file.name);
            println!("{sql}");
            println!();
            continue;
        }

        eprintln!("{} {} {}", arrow(dir), file.version, file.name);
        apply_one(&mut client, file, sql, dir).await?;
    }
    Ok(())
}

fn arrow(dir: Direction) -> &'static str {
    match dir {
        Direction::Up => "==>",
        Direction::Down => "<==",
    }
}

#[derive(Debug)]
struct PlanStep {
    version: String,
}

fn plan(
    local: &BTreeMap<String, MigrationFile>,
    applied: &BTreeMap<String, AppliedRow>,
    dir: Direction,
    to: Option<&str>,
    steps: Option<usize>,
) -> Result<Vec<PlanStep>> {
    match dir {
        Direction::Up => {
            let mut pending: Vec<&str> = local
                .keys()
                .filter(|v| !applied.contains_key(v.as_str()))
                .map(String::as_str)
                .collect();
            pending.sort_unstable();
            if let Some(to) = to {
                pending.retain(|v| *v <= to);
            }
            if let Some(n) = steps {
                pending.truncate(n);
            }
            Ok(pending
                .into_iter()
                .map(|v| PlanStep {
                    version: v.to_string(),
                })
                .collect())
        }
        Direction::Down => {
            let mut applied_versions: Vec<&str> = applied.keys().map(String::as_str).collect();
            applied_versions.sort_unstable();
            // Default down with no flags: roll back exactly one (most recent).
            let target_steps = steps.unwrap_or_else(|| if to.is_some() { usize::MAX } else { 1 });
            let mut to_undo: Vec<&str> = applied_versions.iter().rev().copied().collect();
            if let Some(to) = to {
                to_undo.retain(|v| *v > to);
            }
            to_undo.truncate(target_steps);
            for v in &to_undo {
                if !local.contains_key(*v) {
                    bail!(
                        "cannot roll back {v}: file is missing from migrations/ (was it deleted?)"
                    );
                }
            }
            Ok(to_undo
                .into_iter()
                .map(|v| PlanStep {
                    version: v.to_string(),
                })
                .collect())
        }
    }
}

async fn apply_one(
    client: &mut Client,
    file: &MigrationFile,
    sql: &str,
    dir: Direction,
) -> Result<()> {
    // Tiberius does not interpret `GO`; split into batches ourselves so
    // that statements requiring batch-start position (e.g. `CREATE
    // PROCEDURE`) work across multi-batch migrations.
    let batches = split_batches(sql);

    // BEGIN / COMMIT bracket all batches plus the tracking-table update so
    // a failure mid-migration leaves no partial state.
    client
        .simple_query("BEGIN TRAN sqldev_mig;")
        .await
        .context("begin transaction")?;

    let result: Result<()> = async {
        for batch in &batches {
            if batch.trim().is_empty() {
                continue;
            }
            client
                .simple_query(batch.clone())
                .await
                .with_context(|| format!("execute batch in {} ({})", file.version, file.name))?;
        }

        let tracking_sql = match dir {
            Direction::Up => format!(
                "INSERT INTO dbo.__sqldev_migrations(version, name, checksum) VALUES ({}, {}, {});",
                tsql_str(&file.version),
                tsql_str(&file.name),
                tsql_str(&file.checksum),
            ),
            Direction::Down => format!(
                "DELETE FROM dbo.__sqldev_migrations WHERE version = {};",
                tsql_str(&file.version),
            ),
        };
        client
            .simple_query(tracking_sql)
            .await
            .context("update __sqldev_migrations")?;
        Ok(())
    }
    .await;

    if result.is_err() {
        let _ = client
            .simple_query("IF @@TRANCOUNT > 0 ROLLBACK TRAN;")
            .await;
        return result;
    }

    client
        .simple_query("COMMIT TRAN sqldev_mig;")
        .await
        .context("commit transaction")?;
    Ok(())
}

fn tsql_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 4);
    out.push_str("N'");
    for ch in s.chars() {
        if ch == '\'' {
            out.push_str("''");
        } else {
            out.push(ch);
        }
    }
    out.push('\'');
    out
}

// ---------------------------------------------------------------------------
// File loading + parsing
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct MigrationFile {
    version: String,
    name: String,
    up_sql: Option<String>,
    down_sql: Option<String>,
    checksum: String,
}

impl MigrationFile {
    fn parse_name(file_name: &str) -> Option<MigrationName<'_>> {
        let stem = file_name.strip_suffix(".sql")?;
        let (version, rest) = stem.split_once('_')?;
        if version.is_empty() || !version.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        Some(MigrationName {
            version,
            name: rest,
        })
    }
}

struct MigrationName<'a> {
    version: &'a str,
    name: &'a str,
}

fn load_local(root: &Path) -> Result<BTreeMap<String, MigrationFile>> {
    let dir = root.join("migrations");
    let mut out = BTreeMap::new();
    if !dir.exists() {
        return Ok(out);
    }
    for entry in fs::read_dir(&dir).with_context(|| format!("read {}", dir.display()))? {
        let entry = entry?;
        let file_name = entry.file_name().to_string_lossy().to_string();
        let Some(meta) = MigrationFile::parse_name(&file_name) else {
            continue;
        };
        let body = fs::read_to_string(entry.path())
            .with_context(|| format!("read {}", entry.path().display()))?;
        let (up, down) = split_sections(&body);
        let mut hasher = Sha256::new();
        hasher.update(body.as_bytes());
        let checksum = hex_digest(&hasher.finalize());
        out.insert(
            meta.version.to_string(),
            MigrationFile {
                version: meta.version.to_string(),
                name: meta.name.to_string(),
                up_sql: up,
                down_sql: down,
                checksum,
            },
        );
    }
    Ok(out)
}

fn hex_digest(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// Split a migration body into `Up` and `Down` SQL based on
/// `-- +sqldev Up` / `-- +sqldev Down` directives. When neither is
/// present, the whole body is the Up section.
fn split_sections(body: &str) -> (Option<String>, Option<String>) {
    let mut up: Option<String> = None;
    let mut down: Option<String> = None;
    let mut current: Option<&'static str> = None;
    let mut buf = String::new();
    let mut saw_directive = false;

    for line in body.lines() {
        if let Some(section) = parse_directive(line) {
            saw_directive = true;
            flush(&mut buf, current, &mut up, &mut down);
            current = Some(section);
            continue;
        }
        if current.is_some() {
            buf.push_str(line);
            buf.push('\n');
        }
    }
    flush(&mut buf, current, &mut up, &mut down);

    if !saw_directive {
        // Whole body is the Up section.
        let trimmed = body.trim();
        if !trimmed.is_empty() {
            return (Some(body.to_string()), None);
        }
    }
    (up, down)
}

fn flush(
    buf: &mut String,
    section: Option<&'static str>,
    up: &mut Option<String>,
    down: &mut Option<String>,
) {
    if let Some(s) = section
        && !buf.is_empty()
    {
        let target = match s {
            "up" => up,
            "down" => down,
            _ => return,
        };
        let owned = std::mem::take(buf);
        match target {
            Some(existing) => {
                existing.push('\n');
                existing.push_str(&owned);
            }
            None => *target = Some(owned),
        }
    }
    buf.clear();
}

fn parse_directive(line: &str) -> Option<&'static str> {
    let trimmed = line.trim();
    let rest = trimmed.strip_prefix("--")?.trim_start();
    let rest = rest.strip_prefix("+sqldev")?.trim_start();
    match rest.to_ascii_lowercase().as_str() {
        "up" => Some("up"),
        "down" => Some("down"),
        _ => None,
    }
}

/// Split a SQL string at `GO` batch separators (case-insensitive lines
/// containing only `GO`, optionally surrounded by whitespace).
pub(crate) fn split_batches(sql: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    for line in sql.lines() {
        if line.trim().eq_ignore_ascii_case("GO") {
            if cur.trim().is_empty() {
                cur.clear();
            } else {
                out.push(std::mem::take(&mut cur));
            }
            continue;
        }
        cur.push_str(line);
        cur.push('\n');
    }
    if !cur.trim().is_empty() {
        out.push(cur);
    }
    out
}

// ---------------------------------------------------------------------------
// Server-side state
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct AppliedRow {
    name: String,
    applied_at: String,
    checksum: String,
}

async fn ensure_tracking_table(client: &mut Client) -> Result<()> {
    let sql = "IF OBJECT_ID(N'dbo.__sqldev_migrations', N'U') IS NULL \
        BEGIN \
            CREATE TABLE dbo.__sqldev_migrations ( \
                version NVARCHAR(255) NOT NULL CONSTRAINT PK_sqldev_migrations PRIMARY KEY, \
                name NVARCHAR(500) NOT NULL, \
                applied_at DATETIME2(3) NOT NULL CONSTRAINT DF_sqldev_migrations_applied_at DEFAULT SYSUTCDATETIME(), \
                checksum NVARCHAR(64) NOT NULL \
            ); \
        END";
    client
        .simple_query(sql)
        .await
        .context("create dbo.__sqldev_migrations")?;
    Ok(())
}

async fn load_applied(client: &mut Client) -> Result<BTreeMap<String, AppliedRow>> {
    let rows = client
        .simple_query(
            "SELECT version, name, CONVERT(NVARCHAR(33), applied_at, 126) AS applied_at, checksum \
             FROM dbo.__sqldev_migrations \
             ORDER BY version",
        )
        .await
        .context("load applied migrations")?
        .into_first_result();

    let mut out = BTreeMap::new();
    for row in &rows {
        let version: &str = row.get::<&str, _>(0).unwrap_or_default();
        let name: &str = row.get::<&str, _>(1).unwrap_or_default();
        let applied_at: &str = row.get::<&str, _>(2).unwrap_or_default();
        let checksum: &str = row.get::<&str, _>(3).unwrap_or_default();
        out.insert(
            version.to_string(),
            AppliedRow {
                name: name.to_string(),
                applied_at: applied_at.to_string(),
                checksum: checksum.to_string(),
            },
        );
    }
    Ok(out)
}

async fn acquire_app_lock(client: &mut Client) -> Result<()> {
    // sp_getapplock returns >= 0 on success, < 0 on failure.
    let rows = client
        .simple_query(
            "DECLARE @rc INT; \
             EXEC @rc = sp_getapplock @Resource = N'sqldev:migrate', \
                                      @LockMode = N'Exclusive', \
                                      @LockOwner = N'Session', \
                                      @LockTimeout = 30000; \
             SELECT @rc AS rc;",
        )
        .await
        .context("sp_getapplock")?
        .into_first_result();
    let rc: i32 = rows.first().and_then(|r| r.get::<i32, _>(0)).unwrap_or(-99);
    if rc < 0 {
        bail!(
            "could not acquire sqldev:migrate app lock (sp_getapplock rc={rc}); another migrator may be running"
        );
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_name_accepts_numeric_prefix() {
        let m = MigrationFile::parse_name("0001_baseline.sql").unwrap();
        assert_eq!(m.version, "0001");
        assert_eq!(m.name, "baseline");
    }

    #[test]
    fn parse_name_rejects_non_numeric_prefix() {
        assert!(MigrationFile::parse_name("baseline.sql").is_none());
        assert!(MigrationFile::parse_name("v1_baseline.sql").is_none());
        assert!(MigrationFile::parse_name("0001_baseline.txt").is_none());
    }

    #[test]
    fn split_sections_with_directives() {
        let body = "-- +sqldev Up\nCREATE TABLE t(id INT);\n-- +sqldev Down\nDROP TABLE t;\n";
        let (up, down) = split_sections(body);
        assert!(up.unwrap().contains("CREATE TABLE"));
        assert!(down.unwrap().contains("DROP TABLE"));
    }

    #[test]
    fn split_sections_without_directives_treats_body_as_up() {
        let body = "CREATE TABLE t(id INT);";
        let (up, down) = split_sections(body);
        assert_eq!(up.as_deref(), Some("CREATE TABLE t(id INT);"));
        assert!(down.is_none());
    }

    #[test]
    fn split_batches_at_go() {
        let sql =
            "CREATE TABLE a(id INT);\nGO\nCREATE TABLE b(id INT);\ngo\nINSERT INTO a VALUES(1);\n";
        let batches = split_batches(sql);
        assert_eq!(batches.len(), 3);
        assert!(batches[0].contains("CREATE TABLE a"));
        assert!(batches[1].contains("CREATE TABLE b"));
        assert!(batches[2].contains("INSERT INTO a"));
    }

    #[test]
    fn split_batches_no_separator_returns_single() {
        let sql = "SELECT 1;";
        let batches = split_batches(sql);
        assert_eq!(batches.len(), 1);
    }

    #[test]
    fn sanitize_name_lowercases_and_replaces_separators() {
        assert_eq!(sanitize_name("Add Users-Table"), "add_users_table");
    }

    #[test]
    fn next_version_walks_existing_files() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("0001_baseline.sql"), "").unwrap();
        fs::write(tmp.path().join("0007_skip.sql"), "").unwrap();
        fs::write(tmp.path().join("ignored.txt"), "").unwrap();
        assert_eq!(next_version(tmp.path()).unwrap(), 8);
    }

    #[test]
    fn plan_up_respects_to_and_steps() {
        let mut local = BTreeMap::new();
        for v in ["0001", "0002", "0003", "0004"] {
            local.insert(
                v.to_string(),
                MigrationFile {
                    version: v.to_string(),
                    name: "x".to_string(),
                    up_sql: Some(String::new()),
                    down_sql: None,
                    checksum: String::new(),
                },
            );
        }
        let mut applied = BTreeMap::new();
        applied.insert(
            "0001".to_string(),
            AppliedRow {
                name: "x".into(),
                applied_at: String::new(),
                checksum: String::new(),
            },
        );
        let p = plan(&local, &applied, Direction::Up, Some("0003"), None).unwrap();
        let versions: Vec<_> = p.iter().map(|s| s.version.clone()).collect();
        assert_eq!(versions, vec!["0002", "0003"]);

        let p = plan(&local, &applied, Direction::Up, None, Some(1)).unwrap();
        let versions: Vec<_> = p.iter().map(|s| s.version.clone()).collect();
        assert_eq!(versions, vec!["0002"]);
    }

    #[test]
    fn plan_down_defaults_to_one_step() {
        let mut local = BTreeMap::new();
        for v in ["0001", "0002", "0003"] {
            local.insert(
                v.to_string(),
                MigrationFile {
                    version: v.to_string(),
                    name: "x".to_string(),
                    up_sql: Some(String::new()),
                    down_sql: Some(String::new()),
                    checksum: String::new(),
                },
            );
        }
        let mut applied = BTreeMap::new();
        for v in ["0001", "0002", "0003"] {
            applied.insert(
                v.to_string(),
                AppliedRow {
                    name: "x".into(),
                    applied_at: String::new(),
                    checksum: String::new(),
                },
            );
        }
        let p = plan(&local, &applied, Direction::Down, None, None).unwrap();
        let versions: Vec<_> = p.iter().map(|s| s.version.clone()).collect();
        assert_eq!(versions, vec!["0003"]);

        let p = plan(&local, &applied, Direction::Down, Some("0001"), None).unwrap();
        let versions: Vec<_> = p.iter().map(|s| s.version.clone()).collect();
        assert_eq!(versions, vec!["0003", "0002"]);
    }

    #[test]
    fn tsql_str_doubles_quotes() {
        assert_eq!(tsql_str("o'malley"), "N'o''malley'");
    }
}
