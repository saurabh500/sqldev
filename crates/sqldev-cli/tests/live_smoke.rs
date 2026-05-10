//! Live integration smoke test for the `sqldev` binary.
//!
//! Skipped by default. Run with:
//!
//! ```bash
//! cargo test -p sqldev-cli --test live_smoke -- --ignored
//! ```
//!
//! Requires a reachable SQL Server. The test reads connection details from
//! these env vars, falling back to the defaults shown:
//!
//! - `SQLDEV_LIVE_HOST`     (default `127.0.0.1`)
//! - `SQLDEV_LIVE_PORT`     (default `1433`)
//! - `SQLDEV_LIVE_USER`     (default `SA`)
//! - `SQLDEV_LIVE_PASSWORD` (required)
//!
//! The test creates and drops a database called `sqldev_ci_test`. Anything
//! already in that database will be lost — pick a different name on a
//! shared server.

use std::env;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

const TEST_DB: &str = "sqldev_ci_test";
const READINESS_TIMEOUT: Duration = Duration::from_secs(120);

fn host() -> String {
    env::var("SQLDEV_LIVE_HOST").unwrap_or_else(|_| "127.0.0.1".into())
}

fn port() -> String {
    env::var("SQLDEV_LIVE_PORT").unwrap_or_else(|_| "1433".into())
}

fn user() -> String {
    env::var("SQLDEV_LIVE_USER").unwrap_or_else(|_| "SA".into())
}

fn password() -> String {
    env::var("SQLDEV_LIVE_PASSWORD")
        .expect("set SQLDEV_LIVE_PASSWORD to run live integration tests")
}

fn sqldev() -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_sqldev"));
    // Disable any inherited config discovery so a stray `.sqldev.yml` near
    // CARGO_MANIFEST_DIR doesn't influence the test.
    c.current_dir("/");
    c.env_remove("SQLDEV_HOST");
    c.env_remove("SQLDEV_PORT");
    c.env_remove("SQLDEV_USER");
    c.env_remove("SQLDEV_PASSWORD");
    c.env_remove("SQLDEV_DATABASE");
    c.env_remove("SQLDEV_TRUST_CERT");
    c
}

fn common_conn_flags(cmd: &mut Command, db: &str) {
    cmd.args([
        "--host",
        &host(),
        "--port",
        &port(),
        "--user",
        &user(),
        "--password",
        &password(),
        "--database",
        db,
        "--trust-cert",
        "true",
    ]);
}

fn run_query(db: &str, sql: &str) -> std::process::Output {
    let mut cmd = sqldev();
    cmd.arg("query");
    common_conn_flags(&mut cmd, db);
    cmd.args(["--sql", sql]);
    cmd.output().expect("spawn sqldev query")
}

/// Block until `sqldev query` against `master` succeeds.
fn await_ready() {
    let started = Instant::now();
    loop {
        let out = run_query("master", "SELECT 1 AS ok");
        if out.status.success() {
            return;
        }
        if started.elapsed() > READINESS_TIMEOUT {
            let last_stderr = String::from_utf8_lossy(&out.stderr);
            panic!(
                "SQL Server did not become reachable within {READINESS_TIMEOUT:?}: {last_stderr}"
            );
        }
        std::thread::sleep(Duration::from_secs(2));
    }
}

fn require_success(label: &str, out: &std::process::Output) {
    assert!(
        out.status.success(),
        "{label} failed (exit={:?})\n--- stdout ---\n{}\n--- stderr ---\n{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

fn reset_test_db() {
    require_success(
        "drop test db",
        &run_query(
            "master",
            &format!(
                "IF DB_ID(N'{TEST_DB}') IS NOT NULL BEGIN ALTER DATABASE [{TEST_DB}] SET SINGLE_USER WITH ROLLBACK IMMEDIATE; DROP DATABASE [{TEST_DB}]; END"
            ),
        ),
    );
    require_success(
        "create test db",
        &run_query("master", &format!("CREATE DATABASE [{TEST_DB}]")),
    );
}

fn seed_schema() {
    // `CREATE SCHEMA` must be the only statement in its batch, so issue it
    // separately from the table DDL.
    require_success(
        "create sales schema",
        &run_query(TEST_DB, "CREATE SCHEMA sales"),
    );
    let ddl = r"
        CREATE TABLE sales.Customer (
            Id INT IDENTITY(1,1) NOT NULL CONSTRAINT PK_Customer PRIMARY KEY,
            Email NVARCHAR(255) NOT NULL CONSTRAINT UQ_Customer_Email UNIQUE,
            CONSTRAINT CK_Customer_Email CHECK ([Email] LIKE '%@%')
        );
        CREATE TABLE sales.[Order] (
            Id INT IDENTITY(1,1) NOT NULL CONSTRAINT PK_Order PRIMARY KEY,
            CustomerId INT NOT NULL,
            CONSTRAINT FK_Order_Customer FOREIGN KEY (CustomerId) REFERENCES sales.Customer(Id) ON DELETE CASCADE
        );
        CREATE NONCLUSTERED INDEX IX_Order_CustomerId ON sales.[Order](CustomerId);
    ";
    require_success("seed schema", &run_query(TEST_DB, ddl));
}

#[test]
#[ignore = "live test; requires SQL Server (run with --ignored)"]
fn live_introspect_emits_expected_schema_graph() {
    await_ready();
    reset_test_db();
    seed_schema();

    let mut cmd = sqldev();
    cmd.arg("introspect");
    common_conn_flags(&mut cmd, TEST_DB);
    let out = cmd.output().expect("spawn sqldev introspect");
    require_success("introspect", &out);

    let json: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("introspect output is valid JSON");
    assert_eq!(json["database"], TEST_DB);

    let sales = json["schemas"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["name"] == "sales")
        .expect("sales schema present");
    let table_names: Vec<&str> = sales["tables"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert!(table_names.contains(&"Customer"), "tables: {table_names:?}");
    assert!(table_names.contains(&"Order"), "tables: {table_names:?}");

    // FK was picked up.
    let order = sales["tables"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "Order")
        .unwrap();
    let fks = order["foreign_keys"].as_array().unwrap();
    assert_eq!(fks.len(), 1);
    assert_eq!(fks[0]["name"], "FK_Order_Customer");
    assert_eq!(fks[0]["on_delete"], "CASCADE");
}

#[test]
#[ignore = "live test; requires SQL Server (run with --ignored)"]
fn live_init_writes_baseline_with_real_ddl() {
    await_ready();
    reset_test_db();
    seed_schema();

    let tmp = tempfile::tempdir().expect("tempdir");

    let mut cmd = sqldev();
    cmd.arg("init").arg("--path").arg(tmp.path());
    common_conn_flags(&mut cmd, TEST_DB);
    let out = cmd.output().expect("spawn sqldev init");
    require_success("init", &out);

    let baseline_path = tmp.path().join("migrations").join("0001_baseline.sql");
    assert!(baseline_path.exists(), "{}", baseline_path.display());

    let baseline = std::fs::read_to_string(&baseline_path).unwrap();
    assert!(
        baseline.contains("CREATE SCHEMA [sales]"),
        "missing CREATE SCHEMA in:\n{baseline}"
    );
    assert!(
        baseline.contains("CREATE TABLE [sales].[Customer]"),
        "missing Customer table"
    );
    assert!(
        baseline.contains("CREATE TABLE [sales].[Order]"),
        "missing Order table"
    );
    assert!(
        baseline.contains("CONSTRAINT [PK_Customer] PRIMARY KEY"),
        "missing PK"
    );
    assert!(
        baseline.contains("CONSTRAINT [UQ_Customer_Email] UNIQUE"),
        "missing UNIQUE"
    );
    assert!(
        baseline.contains("CONSTRAINT [CK_Customer_Email] CHECK"),
        "missing CHECK"
    );
    assert!(
        baseline.contains("CONSTRAINT [FK_Order_Customer]"),
        "missing FK"
    );
    assert!(
        baseline.contains("CREATE NONCLUSTERED INDEX [IX_Order_CustomerId]"),
        "missing index"
    );
    assert!(
        baseline.contains("ON DELETE CASCADE"),
        "missing ON DELETE CASCADE"
    );

    // Layout sanity.
    for sub in ["migrations", "seeds", "models"] {
        assert!(tmp.path().join(sub).is_dir(), "{sub} dir not created");
    }
    let yml = std::fs::read_to_string(tmp.path().join(".sqldev.yml")).unwrap();
    assert!(yml.contains(&format!("database: {TEST_DB}")));
    assert!(yml.contains("password: ${SQLDEV_PASSWORD}"));
}

#[test]
#[ignore = "live test; requires SQL Server (run with --ignored)"]
fn live_query_table_format_renders_columns() {
    await_ready();
    reset_test_db();
    require_success(
        "seed",
        &run_query(
            TEST_DB,
            "CREATE TABLE dbo.Demo (Id INT NOT NULL, Name NVARCHAR(20) NOT NULL); INSERT INTO dbo.Demo VALUES (1,'a'),(2,'bee');",
        ),
    );

    let mut cmd = sqldev();
    cmd.arg("query");
    common_conn_flags(&mut cmd, TEST_DB);
    cmd.args([
        "--format",
        "json",
        "--sql",
        "SELECT Id, Name FROM dbo.Demo ORDER BY Id",
    ]);
    let out = cmd.output().expect("spawn query");
    require_success("query json", &out);
    let arr: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let rows = arr.as_array().unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["Id"], 1);
    assert_eq!(rows[1]["Name"], "bee");
}

// --- silence unused-import warnings on platforms that skip ignored tests --

#[allow(dead_code)]
fn _silence_unused(_p: &Path) {}

#[test]
#[ignore = "live test; requires SQL Server (run with --ignored)"]
fn live_migrate_up_status_down_round_trip() {
    await_ready();
    reset_test_db();

    let tmp = tempfile::tempdir().expect("tempdir");
    let migrations = tmp.path().join("migrations");
    std::fs::create_dir_all(&migrations).unwrap();

    // Two migrations exercising both Up and Down sections, plus a multi-batch
    // body separated by `GO`.
    std::fs::write(
        migrations.join("0001_create_widgets.sql"),
        "-- +sqldev Up\n\
         CREATE TABLE dbo.Widget (Id INT NOT NULL CONSTRAINT PK_Widget PRIMARY KEY, Name NVARCHAR(50) NOT NULL);\n\
         GO\n\
         INSERT INTO dbo.Widget (Id, Name) VALUES (1, N'first');\n\
         -- +sqldev Down\n\
         DROP TABLE dbo.Widget;\n",
    )
    .unwrap();
    std::fs::write(
        migrations.join("0002_add_gadget.sql"),
        "-- +sqldev Up\n\
         CREATE TABLE dbo.Gadget (Id INT NOT NULL CONSTRAINT PK_Gadget PRIMARY KEY);\n\
         -- +sqldev Down\n\
         DROP TABLE dbo.Gadget;\n",
    )
    .unwrap();

    // --- migrate up (apply all) ---
    let mut cmd = sqldev();
    cmd.args(["migrate", "up", "--path"]).arg(tmp.path());
    common_conn_flags(&mut cmd, TEST_DB);
    let out = cmd.output().expect("spawn migrate up");
    require_success("migrate up", &out);

    // Both tables exist.
    let q = run_query(
        TEST_DB,
        "SELECT (CASE WHEN OBJECT_ID('dbo.Widget') IS NOT NULL THEN 1 ELSE 0 END) AS w, \
                (CASE WHEN OBJECT_ID('dbo.Gadget') IS NOT NULL THEN 1 ELSE 0 END) AS g, \
                (SELECT COUNT(*) FROM dbo.Widget) AS rows",
    );
    require_success("post-up check", &q);
    let stdout = String::from_utf8_lossy(&q.stdout);
    assert!(stdout.contains('1'), "expected widget to exist: {stdout}");

    // Tracking table records both versions.
    let q = run_query(
        TEST_DB,
        "SELECT version FROM dbo.__sqldev_migrations ORDER BY version",
    );
    require_success("tracking rows", &q);
    let rows = String::from_utf8_lossy(&q.stdout);
    assert!(rows.contains("0001"));
    assert!(rows.contains("0002"));

    // --- migrate up (idempotent) ---
    let mut cmd = sqldev();
    cmd.args(["migrate", "up", "--path"]).arg(tmp.path());
    common_conn_flags(&mut cmd, TEST_DB);
    let out = cmd.output().expect("spawn migrate up #2");
    require_success("migrate up (idempotent)", &out);

    // --- migrate status ---
    let mut cmd = sqldev();
    cmd.args(["migrate", "status", "--path"]).arg(tmp.path());
    common_conn_flags(&mut cmd, TEST_DB);
    let out = cmd.output().expect("spawn status");
    require_success("status", &out);
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.contains("0001"), "status missing 0001:\n{s}");
    assert!(s.contains("0002"), "status missing 0002:\n{s}");
    assert!(s.contains("applied"), "status missing applied label:\n{s}");

    // --- migrate down (default = 1 step) ---
    let mut cmd = sqldev();
    cmd.args(["migrate", "down", "--path"]).arg(tmp.path());
    common_conn_flags(&mut cmd, TEST_DB);
    let out = cmd.output().expect("spawn migrate down");
    require_success("migrate down", &out);

    let q = run_query(
        TEST_DB,
        "SELECT (CASE WHEN OBJECT_ID('dbo.Gadget') IS NOT NULL THEN 1 ELSE 0 END) AS g",
    );
    require_success("post-down check", &q);
    let stdout = String::from_utf8_lossy(&q.stdout);
    assert!(
        stdout.lines().any(|l| l.trim() == "0"),
        "expected Gadget to be dropped:\n{stdout}"
    );

    // --- migrate down --to 0000 (rolls back everything) ---
    let mut cmd = sqldev();
    cmd.args(["migrate", "down", "--to", "0000", "--path"])
        .arg(tmp.path());
    common_conn_flags(&mut cmd, TEST_DB);
    let out = cmd.output().expect("spawn migrate down --to");
    require_success("migrate down --to 0000", &out);

    let q = run_query(TEST_DB, "SELECT COUNT(*) AS n FROM dbo.__sqldev_migrations");
    require_success("count tracking", &q);
    let stdout = String::from_utf8_lossy(&q.stdout);
    assert!(
        stdout.lines().any(|l| l.trim() == "0"),
        "tracking table should be empty:\n{stdout}"
    );
}

#[test]
#[ignore = "live test; requires SQL Server (run with --ignored)"]
fn live_migrate_dry_run_makes_no_changes() {
    await_ready();
    reset_test_db();

    let tmp = tempfile::tempdir().expect("tempdir");
    let migrations = tmp.path().join("migrations");
    std::fs::create_dir_all(&migrations).unwrap();
    std::fs::write(
        migrations.join("0001_dry.sql"),
        "-- +sqldev Up\nCREATE TABLE dbo.ShouldNotExist (Id INT);\n-- +sqldev Down\nDROP TABLE dbo.ShouldNotExist;\n",
    )
    .unwrap();

    let mut cmd = sqldev();
    cmd.args(["migrate", "up", "--dry-run", "--path"])
        .arg(tmp.path());
    common_conn_flags(&mut cmd, TEST_DB);
    let out = cmd.output().expect("spawn dry-run");
    require_success("dry-run", &out);
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(
        s.contains("CREATE TABLE dbo.ShouldNotExist"),
        "dry-run did not echo SQL:\n{s}"
    );

    let q = run_query(
        TEST_DB,
        "SELECT (CASE WHEN OBJECT_ID('dbo.ShouldNotExist') IS NOT NULL THEN 1 ELSE 0 END) AS x",
    );
    require_success("post-dry-run check", &q);
    let stdout = String::from_utf8_lossy(&q.stdout);
    assert!(
        stdout.lines().any(|l| l.trim() == "0"),
        "dry-run should not have created table:\n{stdout}"
    );
}

#[test]
#[ignore = "live test; requires SQL Server (run with --ignored)"]
fn live_diff_output_apply_round_trip_is_empty() {
    await_ready();
    reset_test_db();
    seed_schema();

    let tmp = tempfile::tempdir().expect("tempdir");
    let baseline = tmp.path().join("baseline.json");
    let target = tmp.path().join("target.json");
    let migrations = tmp.path().join("migrations");
    std::fs::create_dir_all(&migrations).unwrap();

    // Ensure the migrations tracking table exists on the *target* side too,
    // so the round-trip diff doesn't see it as a spurious extra table.
    let mut cmd = sqldev();
    cmd.args(["migrate", "status", "--path"]).arg(tmp.path());
    common_conn_flags(&mut cmd, TEST_DB);
    require_success(
        "migrate status (target seed)",
        &cmd.output().expect("spawn status"),
    );

    // 1. Capture target schema (with sales.Customer / sales.[Order]).
    let mut cmd = sqldev();
    cmd.arg("introspect");
    common_conn_flags(&mut cmd, TEST_DB);
    let out = cmd.output().expect("spawn introspect target");
    require_success("introspect target", &out);
    std::fs::write(&target, &out.stdout).unwrap();

    // 2. Reset to empty DB, then create tracking table on the baseline side
    //    too (same reason as above).
    reset_test_db();
    let mut cmd = sqldev();
    cmd.args(["migrate", "status", "--path"]).arg(tmp.path());
    common_conn_flags(&mut cmd, TEST_DB);
    require_success(
        "migrate status (baseline seed)",
        &cmd.output().expect("spawn status"),
    );

    let mut cmd = sqldev();
    cmd.arg("introspect");
    common_conn_flags(&mut cmd, TEST_DB);
    let out = cmd.output().expect("spawn introspect baseline");
    require_success("introspect baseline", &out);
    std::fs::write(&baseline, &out.stdout).unwrap();

    // 3. Generate migration via diff --output.
    let mut cmd = sqldev();
    cmd.args(["diff", "--old"])
        .arg(&baseline)
        .arg("--new")
        .arg(&target)
        .arg("--output")
        .arg(&migrations)
        .args(["--name", "round trip"]);
    let out = cmd.output().expect("spawn diff --output");
    require_success("diff --output", &out);
    let mig_path = migrations.join("0001_round_trip.sql");
    assert!(mig_path.exists(), "{} missing", mig_path.display());

    // 4. Apply via migrate up.
    let mut cmd = sqldev();
    cmd.args(["migrate", "up", "--path"]).arg(tmp.path());
    common_conn_flags(&mut cmd, TEST_DB);
    let out = cmd.output().expect("spawn migrate up");
    require_success("migrate up", &out);

    // 5. Re-introspect and diff vs target → must be empty.
    let mut cmd = sqldev();
    cmd.arg("introspect");
    common_conn_flags(&mut cmd, TEST_DB);
    let out = cmd.output().expect("spawn introspect after apply");
    require_success("introspect after apply", &out);
    let after = tmp.path().join("after.json");
    std::fs::write(&after, &out.stdout).unwrap();

    let mut cmd = sqldev();
    cmd.args(["diff", "--old"])
        .arg(&after)
        .arg("--new")
        .arg(&target)
        .arg("--quiet");
    let out = cmd.output().expect("spawn final diff");
    require_success("final diff", &out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.trim() == "-- no changes",
        "round-trip not empty:\n{stdout}"
    );
}
