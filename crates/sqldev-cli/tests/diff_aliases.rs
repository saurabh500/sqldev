//! Offline integration tests for `--source`/`--target` aliases on `sqldev diff`.

use std::path::Path;
use std::process::Command;

use sqldev_core::{
    Column, KeyConstraint, SchemaGraph, SchemaNode, Table, schema::SCHEMA_GRAPH_VERSION,
};

fn sqldev() -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_sqldev"));
    c.current_dir("/");
    for k in [
        "SQLDEV_HOST",
        "SQLDEV_PORT",
        "SQLDEV_USER",
        "SQLDEV_PASSWORD",
        "SQLDEV_DATABASE",
        "SQLDEV_TRUST_CERT",
    ] {
        c.env_remove(k);
    }
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

fn customer_table() -> Table {
    Table {
        name: "Customer".into(),
        columns: vec![Column {
            name: "Id".into(),
            type_name: "int".into(),
            nullable: false,
            identity: true,
            is_uddt: false,
            udt_schema: None,
            base_type: None,
            default: None,
            computed: None,
        }],
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

#[test]
fn diff_source_target_aliases_match_old_new() {
    let tmp = tempfile::tempdir().unwrap();
    let old_path = tmp.path().join("old.json");
    let new_path = tmp.path().join("new.json");

    let old = empty_graph("AW");
    let mut new = empty_graph("AW");
    new.schemas[0].tables.push(customer_table());
    write_graph(&old_path, &old);
    write_graph(&new_path, &new);

    // Use --source / --target in place of --old / --new.
    let mut cmd = sqldev();
    cmd.args(["diff", "--source"])
        .arg(&old_path)
        .arg("--target")
        .arg(&new_path);
    let out = cmd.output().expect("spawn diff");
    assert!(
        out.status.success(),
        "diff failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("CREATE TABLE [dbo].[Customer]"),
        "expected CREATE in:\n{stdout}"
    );
}
