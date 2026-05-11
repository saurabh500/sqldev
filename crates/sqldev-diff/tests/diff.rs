//! Integration tests for `sqldev_diff::diff`.
//!
//! Each test builds two `SchemaGraph`s programmatically and asserts the
//! ordered T-SQL output. We assert both shape (which kinds of statements
//! appear) and ordering invariants (drops before adds, FKs added last).

use sqldev_core::{
    CheckConstraint, Column, ForeignKey, Index, KeyConstraint, Routine, SchemaGraph, SchemaNode,
    Table, Trigger, View, schema::SCHEMA_GRAPH_VERSION,
};
use sqldev_diff::diff;

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

fn table_with_pk(name: &str) -> Table {
    Table {
        name: name.into(),
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
            name: format!("PK_{name}"),
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

fn dbo_with(tables: Vec<Table>) -> SchemaGraph {
    let mut g = empty_graph("AdventureWorks");
    g.schemas[0].tables = tables;
    g
}

#[test]
fn no_changes_emits_no_statements() {
    let g = dbo_with(vec![table_with_pk("Customer")]);
    let r = diff(&g, &g);
    assert!(r.statements.is_empty(), "got {:?}", r.statements);
}

#[test]
fn add_table_emits_create() {
    let old = empty_graph("AW");
    let new = dbo_with(vec![table_with_pk("Customer")]);
    let r = diff(&old, &new);
    assert!(
        r.statements
            .iter()
            .any(|s| s.starts_with("CREATE TABLE [dbo].[Customer]")),
        "got {:?}",
        r.statements
    );
}

#[test]
fn drop_table_emits_drop() {
    let old = dbo_with(vec![table_with_pk("Customer")]);
    let new = empty_graph("AW");
    let r = diff(&old, &new);
    assert!(
        r.statements
            .contains(&"DROP TABLE [dbo].[Customer];".into())
    );
}

#[test]
fn add_column() {
    let old = dbo_with(vec![table_with_pk("Customer")]);
    let mut new_t = table_with_pk("Customer");
    new_t.columns.push(col("Email", "nvarchar(256)", true));
    let new = dbo_with(vec![new_t]);
    let r = diff(&old, &new);
    assert!(
        r.statements
            .iter()
            .any(|s| s.contains("ADD [Email] nvarchar(256)")),
        "got {:?}",
        r.statements
    );
}

#[test]
fn drop_column() {
    let mut old_t = table_with_pk("Customer");
    old_t.columns.push(col("Email", "nvarchar(256)", true));
    let old = dbo_with(vec![old_t]);
    let new = dbo_with(vec![table_with_pk("Customer")]);
    let r = diff(&old, &new);
    assert!(
        r.statements
            .contains(&"ALTER TABLE [dbo].[Customer] DROP COLUMN [Email];".into())
    );
}

#[test]
fn alter_column_type_and_nullability() {
    let old = dbo_with(vec![table_with_pk("Customer")]);
    let mut new_t = table_with_pk("Customer");
    new_t.columns[1] = col("Name", "nvarchar(200)", true);
    let new = dbo_with(vec![new_t]);
    let r = diff(&old, &new);
    assert!(
        r.statements
            .iter()
            .any(|s| s == "ALTER TABLE [dbo].[Customer] ALTER COLUMN [Name] nvarchar(200) NULL;"),
        "got {:?}",
        r.statements
    );
}

#[test]
fn add_check_constraint() {
    let old = dbo_with(vec![table_with_pk("Customer")]);
    let mut new_t = table_with_pk("Customer");
    new_t.check_constraints.push(CheckConstraint {
        name: "CK_Customer_NameLen".into(),
        expression: "(len([Name])>0)".into(),
    });
    let new = dbo_with(vec![new_t]);
    let r = diff(&old, &new);
    assert!(
        r.statements
            .iter()
            .any(|s| s.contains("ADD CONSTRAINT [CK_Customer_NameLen] CHECK (len([Name])>0)")),
        "got {:?}",
        r.statements
    );
}

#[test]
fn drop_then_add_constraint_when_changed() {
    let mut old_t = table_with_pk("Customer");
    old_t.check_constraints.push(CheckConstraint {
        name: "CK".into(),
        expression: "(len([Name])>0)".into(),
    });
    let old = dbo_with(vec![old_t]);
    let mut new_t = table_with_pk("Customer");
    new_t.check_constraints.push(CheckConstraint {
        name: "CK".into(),
        expression: "(len([Name])>1)".into(),
    });
    let new = dbo_with(vec![new_t]);
    let r = diff(&old, &new);
    let drop_idx = r
        .statements
        .iter()
        .position(|s| s == "ALTER TABLE [dbo].[Customer] DROP CONSTRAINT [CK];")
        .expect("drop");
    let add_idx = r
        .statements
        .iter()
        .position(|s| s.contains("ADD CONSTRAINT [CK] CHECK (len([Name])>1)"))
        .expect("add");
    assert!(
        drop_idx < add_idx,
        "drop must precede add: {:?}",
        r.statements
    );
}

#[test]
fn add_index_with_include_and_filter() {
    let old = dbo_with(vec![table_with_pk("Customer")]);
    let mut new_t = table_with_pk("Customer");
    new_t.indexes.push(Index {
        name: "IX_Customer_Name".into(),
        columns: vec!["Name".into()],
        included_columns: vec!["Id".into()],
        is_unique: false,
        is_clustered: false,
        filter: Some("([Name] IS NOT NULL)".into()),
    });
    let new = dbo_with(vec![new_t]);
    let r = diff(&old, &new);
    assert!(
        r.statements.iter().any(|s| s
            == "CREATE NONCLUSTERED INDEX [IX_Customer_Name] ON [dbo].[Customer] ([Name]) INCLUDE ([Id]) WHERE ([Name] IS NOT NULL);"),
        "got {:?}",
        r.statements
    );
}

#[test]
fn fk_added_after_referenced_table_created() {
    // Both tables new; FK from Order -> Customer must come last.
    let old = empty_graph("AW");
    let mut order = table_with_pk("Order");
    order.columns.push(col("CustomerId", "int", false));
    order.foreign_keys.push(ForeignKey {
        name: "FK_Order_Customer".into(),
        columns: vec!["CustomerId".into()],
        referenced_schema: "dbo".into(),
        referenced_table: "Customer".into(),
        referenced_columns: vec!["Id".into()],
        on_delete: "NO_ACTION".into(),
        on_update: "NO_ACTION".into(),
    });
    let new = dbo_with(vec![table_with_pk("Customer"), order]);
    let r = diff(&old, &new);
    let create_customer = r
        .statements
        .iter()
        .position(|s| s.starts_with("CREATE TABLE [dbo].[Customer]"))
        .expect("create customer");
    let create_order = r
        .statements
        .iter()
        .position(|s| s.starts_with("CREATE TABLE [dbo].[Order]"))
        .expect("create order");
    let add_fk = r
        .statements
        .iter()
        .position(|s| s.contains("ADD CONSTRAINT [FK_Order_Customer]"))
        .expect("add fk");
    assert!(create_customer < add_fk);
    assert!(create_order < add_fk);
}

#[test]
fn fk_dropped_before_table_dropped() {
    let mut order = table_with_pk("Order");
    order.foreign_keys.push(ForeignKey {
        name: "FK_Order_Customer".into(),
        columns: vec!["Id".into()],
        referenced_schema: "dbo".into(),
        referenced_table: "Customer".into(),
        referenced_columns: vec!["Id".into()],
        on_delete: "NO_ACTION".into(),
        on_update: "NO_ACTION".into(),
    });
    let old = dbo_with(vec![table_with_pk("Customer"), order]);
    let new = empty_graph("AW");
    let r = diff(&old, &new);
    let drop_fk = r
        .statements
        .iter()
        .position(|s| s.contains("DROP CONSTRAINT [FK_Order_Customer]"))
        .expect("drop fk");
    let drop_customer = r
        .statements
        .iter()
        .position(|s| s == "DROP TABLE [dbo].[Customer];")
        .expect("drop customer");
    assert!(drop_fk < drop_customer);
}

#[test]
fn create_new_schema() {
    let old = empty_graph("AW");
    let mut new = empty_graph("AW");
    new.schemas.push(SchemaNode {
        name: "sales".into(),
        tables: vec![table_with_pk("Quote")],
        views: vec![],
        procedures: vec![],
        functions: vec![],
        types: vec![],
    });
    let r = diff(&old, &new);
    let create_schema = r
        .statements
        .iter()
        .position(|s| s.contains("CREATE SCHEMA [sales]"))
        .expect("create schema");
    let create_quote = r
        .statements
        .iter()
        .position(|s| s.starts_with("CREATE TABLE [sales].[Quote]"))
        .expect("create table");
    assert!(create_schema < create_quote);
}

#[test]
fn view_body_change_drops_and_recreates() {
    let mut old = empty_graph("AW");
    old.schemas[0].views.push(View {
        name: "v_active".into(),
        definition: Some("CREATE VIEW v_active AS SELECT 1".into()),
    });
    let mut new = empty_graph("AW");
    new.schemas[0].views.push(View {
        name: "v_active".into(),
        definition: Some("CREATE VIEW v_active AS SELECT 2".into()),
    });
    let r = diff(&old, &new);
    let drop_idx = r
        .statements
        .iter()
        .position(|s| s == "DROP VIEW [dbo].[v_active];")
        .expect("drop view");
    let create_idx = r
        .statements
        .iter()
        .position(|s| s.contains("SELECT 2"))
        .expect("create view");
    assert!(drop_idx < create_idx);
}

#[test]
fn proc_added_emits_create() {
    let old = empty_graph("AW");
    let mut new = empty_graph("AW");
    new.schemas[0].procedures.push(Routine {
        name: "p_get".into(),
        kind: "procedure".into(),
        definition: Some("CREATE PROCEDURE p_get AS SELECT 1".into()),
    });
    let r = diff(&old, &new);
    assert!(
        r.statements
            .iter()
            .any(|s| s.contains("CREATE PROCEDURE p_get")),
        "got {:?}",
        r.statements
    );
}

#[test]
fn function_dropped_emits_drop() {
    let mut old = empty_graph("AW");
    old.schemas[0].functions.push(Routine {
        name: "f_x".into(),
        kind: "scalar-function".into(),
        definition: Some("CREATE FUNCTION f_x() RETURNS INT AS BEGIN RETURN 1 END".into()),
    });
    let new = empty_graph("AW");
    let r = diff(&old, &new);
    assert!(r.statements.contains(&"DROP FUNCTION [dbo].[f_x];".into()));
}

#[test]
fn trigger_change_drops_and_recreates() {
    let mut old_t = table_with_pk("Customer");
    old_t.triggers.push(Trigger {
        name: "tr_log".into(),
        timing: "AFTER".into(),
        is_disabled: false,
        definition: Some("CREATE TRIGGER tr_log ON Customer AFTER INSERT AS SELECT 1".into()),
    });
    let mut new_t = table_with_pk("Customer");
    new_t.triggers.push(Trigger {
        name: "tr_log".into(),
        timing: "AFTER".into(),
        is_disabled: false,
        definition: Some("CREATE TRIGGER tr_log ON Customer AFTER INSERT AS SELECT 2".into()),
    });
    let old = dbo_with(vec![old_t]);
    let new = dbo_with(vec![new_t]);
    let r = diff(&old, &new);
    let drop_idx = r
        .statements
        .iter()
        .position(|s| s == "DROP TRIGGER [dbo].[tr_log];")
        .expect("drop trigger");
    let create_idx = r
        .statements
        .iter()
        .position(|s| s.contains("SELECT 2"))
        .expect("create trigger");
    assert!(drop_idx < create_idx);
}

#[test]
fn fk_with_cascade_emits_on_delete_clause() {
    let old = empty_graph("AW");
    let mut order = table_with_pk("Order");
    order.columns.push(col("CustomerId", "int", false));
    order.foreign_keys.push(ForeignKey {
        name: "FK".into(),
        columns: vec!["CustomerId".into()],
        referenced_schema: "dbo".into(),
        referenced_table: "Customer".into(),
        referenced_columns: vec!["Id".into()],
        on_delete: "CASCADE".into(),
        on_update: "NO_ACTION".into(),
    });
    let new = dbo_with(vec![table_with_pk("Customer"), order]);
    let r = diff(&old, &new);
    assert!(
        r.statements.iter().any(|s| s.contains("ON DELETE CASCADE")),
        "got {:?}",
        r.statements
    );
}

#[test]
fn drop_default_warns() {
    let mut old_t = table_with_pk("Customer");
    old_t.columns[1].default = Some("N''".into());
    let old = dbo_with(vec![old_t]);
    let new = dbo_with(vec![table_with_pk("Customer")]);
    let r = diff(&old, &new);
    assert!(
        r.warnings.iter().any(|w| w.contains("dropping default")),
        "warnings: {:?}",
        r.warnings
    );
}

#[test]
fn add_table_with_uddt_renders_owning_schema() {
    // A column referencing a UDDT defined under a non-`dbo` schema must
    // emit `[<udt_schema>].[<type_name>]`, not `[dbo].[<type_name>]`.
    let old = empty_graph("AW");
    let mut new = empty_graph("AW");
    let mut t = table_with_pk("AuditLog");
    t.columns.push(Column {
        name: "Actor".into(),
        type_name: "ShortName".into(),
        nullable: false,
        identity: false,
        is_uddt: true,
        udt_schema: Some("audit".into()),
        base_type: Some("nvarchar(64)".into()),
        default: None,
        computed: None,
    });
    new.schemas[0].tables = vec![t];
    let r = diff(&old, &new);
    let create = r
        .statements
        .iter()
        .find(|s| s.starts_with("CREATE TABLE [dbo].[AuditLog]"))
        .unwrap_or_else(|| panic!("missing CREATE TABLE; got {:?}", r.statements));
    assert!(
        create.contains("[audit].[ShortName]"),
        "expected [audit].[ShortName] in:\n{create}"
    );
    assert!(
        !create.contains("[dbo].[ShortName]"),
        "must not fall back to dbo:\n{create}"
    );
}

#[test]
fn missing_udt_schema_falls_back_to_dbo() {
    // Pre-#37 snapshots may have `udt_schema = None`. Preserve the
    // historical behaviour of assuming `dbo` so old diff inputs keep
    // producing the same DDL.
    let old = empty_graph("AW");
    let mut new = empty_graph("AW");
    let mut t = table_with_pk("AuditLog");
    t.columns.push(Column {
        name: "Actor".into(),
        type_name: "ShortName".into(),
        nullable: false,
        identity: false,
        is_uddt: true,
        udt_schema: None,
        base_type: Some("nvarchar(64)".into()),
        default: None,
        computed: None,
    });
    new.schemas[0].tables = vec![t];
    let r = diff(&old, &new);
    let create = r
        .statements
        .iter()
        .find(|s| s.starts_with("CREATE TABLE [dbo].[AuditLog]"))
        .unwrap();
    assert!(create.contains("[dbo].[ShortName]"), "got:\n{create}");
}
