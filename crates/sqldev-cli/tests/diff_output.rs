//! Integration tests for `sqldev diff --output`.
//!
//! Drives the compiled `sqldev` binary against synthetic schema-graph
//! JSON files on disk. No SQL Server required.

use std::path::Path;
use std::process::Command;

use sqldev_core::{
    Column, KeyConstraint, SchemaGraph, SchemaNode, Table, schema::SCHEMA_GRAPH_VERSION,
};

fn sqldev() -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_sqldev"));
    c.current_dir("/");
    c.env_remove("SQLDEV_HOST");
    c.env_remove("SQLDEV_PORT");
    c.env_remove("SQLDEV_USER");
    c.env_remove("SQLDEV_PASSWORD");
    c.env_remove("SQLDEV_DATABASE");
    c.env_remove("SQLDEV_TRUST_CERT");
    c
}

fn empty_graph(db: &str) -> SchemaGraph {
    SchemaGraph {
        version: SCHEMA_GRAPH_VERSION.to_string(),
        database: db.to_string(),
        schemas: vec![SchemaNode {
            name: "dbo".into(),
            tables: vec![],
            views: vec![],
            procedures: vec![],
            functions: vec![],
            types: vec![],
        }],
        warnings: vec![],
    }
}

fn col(name: &str, type_name: &str, nullable: bool) -> Column {
    Column {
        name: name.into(),
        type_name: type_name.into(),
        nullable,
        identity: false,
        is_uddt: false,
        udt_schema: None,
        base_type: None,
        default: None,
        computed: None,
    }
}

fn customer_table() -> Table {
    Table {
        name: "Customer".into(),
        columns: vec![
            Column {
                name: "Id".into(),
                type_name: "int".into(),
                nullable: false,
                identity: true,
                is_uddt: false,
                udt_schema: None,
                base_type: None,
                default: None,
                computed: None,
            },
            col("Name", "nvarchar(100)", false),
        ],
        primary_key: Some(KeyConstraint {
            name: "PK_Customer".into(),
            columns: vec!["Id".into()],
            clustered: true,
        }),
        unique_constraints: vec![],
        check_constraints: vec![],
        foreign_keys: vec![],
        indexes: vec![],
        triggers: vec![],
    }
}

fn write_graph(path: &Path, g: &SchemaGraph) {
    std::fs::write(path, serde_json::to_string_pretty(g).unwrap()).unwrap();
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

#[test]
fn diff_output_writes_numbered_migration_with_up_and_down() {
    let tmp = tempfile::tempdir().unwrap();
    let old_path = tmp.path().join("old.json");
    let new_path = tmp.path().join("new.json");
    let mig_dir = tmp.path().join("migrations");

    let old = empty_graph("AW");
    let mut new = empty_graph("AW");
    new.schemas[0].tables.push(customer_table());

    write_graph(&old_path, &old);
    write_graph(&new_path, &new);

    let mut cmd = sqldev();
    cmd.args(["diff", "--old"])
        .arg(&old_path)
        .arg("--new")
        .arg(&new_path)
        .arg("--output")
        .arg(&mig_dir)
        .args(["--name", "Add Customer"]);
    let out = cmd.output().expect("spawn diff");
    require_success("diff --output", &out);

    let entries: Vec<_> = std::fs::read_dir(&mig_dir).unwrap().collect();
    assert_eq!(entries.len(), 1, "expected exactly one migration file");
    let path = entries[0].as_ref().unwrap().path();
    let name = path.file_name().unwrap().to_string_lossy().into_owned();
    assert!(
        name.starts_with("0001_") && name.to_ascii_lowercase().ends_with(".sql"),
        "unexpected name: {name}"
    );
    assert!(name.contains("add_customer"), "unexpected name: {name}");

    let body = std::fs::read_to_string(&path).unwrap();
    let up = body.find("-- +sqldev Up").expect("missing Up marker");
    let down = body.find("-- +sqldev Down").expect("missing Down marker");
    assert!(up < down, "Up marker must precede Down");

    let up_section = &body[up..down];
    let down_section = &body[down..];
    assert!(
        up_section.contains("CREATE TABLE [dbo].[Customer]"),
        "Up missing CREATE: {up_section}"
    );
    assert!(
        down_section.contains("DROP TABLE [dbo].[Customer]"),
        "Down missing DROP: {down_section}"
    );
}

#[test]
fn diff_output_picks_next_version_when_files_exist() {
    let tmp = tempfile::tempdir().unwrap();
    let mig_dir = tmp.path().join("migrations");
    std::fs::create_dir_all(&mig_dir).unwrap();
    std::fs::write(mig_dir.join("0001_baseline.sql"), "-- +sqldev Up\n").unwrap();
    std::fs::write(mig_dir.join("0007_skip.sql"), "-- +sqldev Up\n").unwrap();
    std::fs::write(mig_dir.join("not_a_migration.txt"), "noise").unwrap();

    let old_path = tmp.path().join("old.json");
    let new_path = tmp.path().join("new.json");
    let old = empty_graph("AW");
    let mut new = empty_graph("AW");
    new.schemas[0].tables.push(customer_table());
    write_graph(&old_path, &old);
    write_graph(&new_path, &new);

    let mut cmd = sqldev();
    cmd.args(["diff", "--old"])
        .arg(&old_path)
        .arg("--new")
        .arg(&new_path)
        .arg("--output")
        .arg(&mig_dir);
    let out = cmd.output().expect("spawn diff");
    require_success("diff --output", &out);

    assert!(
        mig_dir.join("0008_diff.sql").exists(),
        "expected 0008_diff.sql; got: {:?}",
        std::fs::read_dir(&mig_dir)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect::<Vec<_>>()
    );
}

#[test]
fn diff_output_no_changes_writes_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let mig_dir = tmp.path().join("migrations");
    let old_path = tmp.path().join("old.json");
    let new_path = tmp.path().join("new.json");
    let g = {
        let mut g = empty_graph("AW");
        g.schemas[0].tables.push(customer_table());
        g
    };
    write_graph(&old_path, &g);
    write_graph(&new_path, &g);

    let mut cmd = sqldev();
    cmd.args(["diff", "--old"])
        .arg(&old_path)
        .arg("--new")
        .arg(&new_path)
        .arg("--output")
        .arg(&mig_dir);
    let out = cmd.output().expect("spawn diff");
    require_success("diff --output (no changes)", &out);

    assert!(
        !mig_dir.exists() || std::fs::read_dir(&mig_dir).unwrap().next().is_none(),
        "no migration file should have been written"
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("no changes"),
        "expected no-changes msg: {stdout}"
    );
}

#[test]
fn diff_output_refuses_when_warnings_present() {
    // Drop a default → emits a warning.
    let tmp = tempfile::tempdir().unwrap();
    let mig_dir = tmp.path().join("migrations");
    let old_path = tmp.path().join("old.json");
    let new_path = tmp.path().join("new.json");

    let mut old = empty_graph("AW");
    let mut t = customer_table();
    t.columns[1].default = Some("N''".into());
    old.schemas[0].tables.push(t);

    let mut new = empty_graph("AW");
    new.schemas[0].tables.push(customer_table());

    write_graph(&old_path, &old);
    write_graph(&new_path, &new);

    let mut cmd = sqldev();
    cmd.args(["diff", "--old"])
        .arg(&old_path)
        .arg("--new")
        .arg(&new_path)
        .arg("--output")
        .arg(&mig_dir);
    let out = cmd.output().expect("spawn diff");
    assert!(
        !out.status.success(),
        "expected failure due to warnings; stdout={} stderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("refusing to write migration"),
        "unexpected stderr: {stderr}"
    );

    // With --allow-warnings it succeeds.
    let mut cmd = sqldev();
    cmd.args(["diff", "--old"])
        .arg(&old_path)
        .arg("--new")
        .arg(&new_path)
        .arg("--output")
        .arg(&mig_dir)
        .arg("--allow-warnings");
    let out = cmd.output().expect("spawn diff allow-warnings");
    require_success("diff --output --allow-warnings", &out);
    assert!(mig_dir.join("0001_diff.sql").exists());
}
