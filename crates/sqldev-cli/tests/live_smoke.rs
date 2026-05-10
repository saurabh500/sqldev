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
    let ddl = r"
        CREATE SCHEMA sales;
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
