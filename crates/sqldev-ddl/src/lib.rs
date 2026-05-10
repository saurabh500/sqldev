//! T-SQL DDL emission from a [`SchemaGraph`].
//!
//! The emitter produces deterministic, line-stable output. Output ordering
//! mirrors the resolution order used by [`spike-diff`]'s "create from
//! scratch" phases:
//!
//! 1. `CREATE SCHEMA` for every non-`dbo` schema (alphabetical).
//! 2. `CREATE TYPE` for every UDDT (per schema, alphabetical).
//! 3. `CREATE TABLE` (with inline PK / UNIQUE / CHECK), alphabetical by
//!    `(schema, table)`. Self-referencing FKs are emitted in phase 5.
//! 4. `CREATE [UNIQUE] [CLUSTERED] INDEX`, alphabetical.
//! 5. `ALTER TABLE ... ADD CONSTRAINT ... FOREIGN KEY`, alphabetical.
//!
//! Views, procedures, functions and triggers are intentionally omitted from
//! the baseline today — those bodies are reproduced via separate spike
//! tooling and aren't part of M1's "structural baseline" scope.

#![forbid(unsafe_code)]

use sqldev_core::{
    CheckConstraint, Column, ForeignKey, Index, KeyConstraint, SchemaGraph, Table, UserDefinedType,
};
use std::collections::BTreeMap;
use std::fmt::Write as _;

/// Render a baseline T-SQL script for `graph`.
///
/// The script is safe to apply against an empty database; statements are
/// terminated with `;` and separated by blank lines so the output diffs
/// cleanly.
#[must_use]
pub fn baseline_ddl(graph: &SchemaGraph) -> String {
    let mut out: Vec<String> = Vec::new();

    // Index schemas by name for stable ordering.
    let mut schemas: BTreeMap<&str, _> = BTreeMap::new();
    for s in &graph.schemas {
        schemas.insert(s.name.as_str(), s);
    }

    // 1. CREATE SCHEMA (skip dbo — always present).
    for name in schemas.keys() {
        if *name == "dbo" {
            continue;
        }
        out.push(format!(
            "IF SCHEMA_ID(N'{name}') IS NULL EXEC(N'CREATE SCHEMA [{name}]');"
        ));
    }

    // 2. CREATE TYPE for UDDTs.
    for (sname, s) in &schemas {
        let mut types: Vec<&UserDefinedType> = s.types.iter().collect();
        types.sort_by(|a, b| a.name.cmp(&b.name));
        for t in types {
            out.push(create_type_sql(sname, t));
        }
    }

    // 3. CREATE TABLE.
    for (sname, s) in &schemas {
        let mut tables: Vec<&Table> = s.tables.iter().collect();
        tables.sort_by(|a, b| a.name.cmp(&b.name));
        for t in tables {
            out.push(create_table_sql(sname, t));
        }
    }

    // 4. CREATE INDEX (non-PK / non-UQ; PK/UQ are emitted inline).
    for (sname, s) in &schemas {
        let mut tables: Vec<&Table> = s.tables.iter().collect();
        tables.sort_by(|a, b| a.name.cmp(&b.name));
        for t in tables {
            let mut indexes: Vec<&Index> = t.indexes.iter().collect();
            indexes.sort_by(|a, b| a.name.cmp(&b.name));
            for ix in indexes {
                out.push(create_index_sql(sname, &t.name, ix));
            }
        }
    }

    // 5. ALTER TABLE ... ADD CONSTRAINT ... FOREIGN KEY.
    for (sname, s) in &schemas {
        let mut tables: Vec<&Table> = s.tables.iter().collect();
        tables.sort_by(|a, b| a.name.cmp(&b.name));
        for t in tables {
            let mut fks: Vec<&ForeignKey> = t.foreign_keys.iter().collect();
            fks.sort_by(|a, b| a.name.cmp(&b.name));
            for fk in fks {
                out.push(create_fk_sql(sname, &t.name, fk));
            }
        }
    }

    out.join("\n\n") + "\n"
}

// --- Statement emitters --------------------------------------------------

fn col_type(c: &Column) -> String {
    if c.is_uddt {
        // UDDTs are emitted as `[schema].[name]`. The schema graph today
        // doesn't carry the UDDT's owning schema on the column, so we
        // assume `dbo`. This matches the spike-diff behaviour and the
        // common case.
        format!("[dbo].[{}]", c.type_name)
    } else {
        c.type_name.clone()
    }
}

fn col_definition(c: &Column) -> String {
    if let Some(expr) = &c.computed {
        return format!("[{}] AS {expr}", c.name);
    }
    let mut parts = vec![format!("[{}]", c.name), col_type(c)];
    if c.identity {
        parts.push("IDENTITY(1,1)".into());
    }
    parts.push(if c.nullable {
        "NULL".into()
    } else {
        "NOT NULL".into()
    });
    if let Some(d) = &c.default {
        parts.push(format!("DEFAULT ({d})"));
    }
    parts.join(" ")
}

fn create_type_sql(schema: &str, t: &UserDefinedType) -> String {
    let null = if t.nullable { "NULL" } else { "NOT NULL" };
    format!(
        "CREATE TYPE [{schema}].[{}] FROM {} {null};",
        t.name, t.base_type
    )
}

fn create_table_sql(schema: &str, t: &Table) -> String {
    let mut lines: Vec<String> = t
        .columns
        .iter()
        .map(|c| format!("    {}", col_definition(c)))
        .collect();
    if let Some(pk) = &t.primary_key {
        lines.push(format!("    {}", inline_key("PRIMARY KEY", pk)));
    }
    let mut uqs: Vec<&KeyConstraint> = t.unique_constraints.iter().collect();
    uqs.sort_by(|a, b| a.name.cmp(&b.name));
    for uq in uqs {
        lines.push(format!("    {}", inline_key("UNIQUE", uq)));
    }
    let mut ccs: Vec<&CheckConstraint> = t.check_constraints.iter().collect();
    ccs.sort_by(|a, b| a.name.cmp(&b.name));
    for cc in ccs {
        lines.push(format!(
            "    CONSTRAINT [{}] CHECK {}",
            cc.name, cc.expression
        ));
    }
    format!(
        "CREATE TABLE [{schema}].[{}] (\n{}\n);",
        t.name,
        lines.join(",\n")
    )
}

fn inline_key(kind: &str, k: &KeyConstraint) -> String {
    let cols = k
        .columns
        .iter()
        .map(|c| format!("[{c}]"))
        .collect::<Vec<_>>()
        .join(", ");
    let clu = if k.clustered {
        "CLUSTERED"
    } else {
        "NONCLUSTERED"
    };
    format!("CONSTRAINT [{}] {kind} {clu} ({cols})", k.name)
}

fn create_index_sql(schema: &str, table: &str, ix: &Index) -> String {
    let unique = if ix.is_unique { "UNIQUE " } else { "" };
    let clustered = if ix.is_clustered {
        "CLUSTERED "
    } else {
        "NONCLUSTERED "
    };
    let cols = ix
        .columns
        .iter()
        .map(|c| format!("[{c}]"))
        .collect::<Vec<_>>()
        .join(", ");
    let mut sql = format!(
        "CREATE {unique}{clustered}INDEX [{}] ON [{schema}].[{table}] ({cols})",
        ix.name
    );
    if !ix.included_columns.is_empty() {
        let inc = ix
            .included_columns
            .iter()
            .map(|c| format!("[{c}]"))
            .collect::<Vec<_>>()
            .join(", ");
        let _ = write!(sql, " INCLUDE ({inc})");
    }
    if let Some(f) = &ix.filter {
        let _ = write!(sql, " WHERE {f}");
    }
    sql.push(';');
    sql
}

fn create_fk_sql(schema: &str, table: &str, fk: &ForeignKey) -> String {
    let cols = fk
        .columns
        .iter()
        .map(|c| format!("[{c}]"))
        .collect::<Vec<_>>()
        .join(", ");
    let refcols = fk
        .referenced_columns
        .iter()
        .map(|c| format!("[{c}]"))
        .collect::<Vec<_>>()
        .join(", ");
    let mut sql = format!(
        "ALTER TABLE [{schema}].[{table}] ADD CONSTRAINT [{}] FOREIGN KEY ({cols}) REFERENCES [{}].[{}] ({refcols})",
        fk.name, fk.referenced_schema, fk.referenced_table
    );
    if fk.on_delete != "NO_ACTION" && !fk.on_delete.is_empty() {
        let _ = write!(sql, " ON DELETE {}", fk.on_delete.replace('_', " "));
    }
    if fk.on_update != "NO_ACTION" && !fk.on_update.is_empty() {
        let _ = write!(sql, " ON UPDATE {}", fk.on_update.replace('_', " "));
    }
    sql.push(';');
    sql
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqldev_core::{
        CheckConstraint, Column, ForeignKey, Index, KeyConstraint, SchemaGraph, SchemaNode, Table,
        UserDefinedType, schema::SCHEMA_GRAPH_VERSION,
    };

    fn col(name: &str, ty: &str, nullable: bool) -> Column {
        Column {
            name: name.into(),
            type_name: ty.into(),
            nullable,
            identity: false,
            is_uddt: false,
            base_type: None,
            default: None,
            computed: None,
        }
    }

    fn graph_with(schema: &str, tables: Vec<Table>, types: Vec<UserDefinedType>) -> SchemaGraph {
        SchemaGraph {
            version: SCHEMA_GRAPH_VERSION.into(),
            database: "test".into(),
            schemas: vec![SchemaNode {
                name: schema.into(),
                tables,
                views: vec![],
                procedures: vec![],
                functions: vec![],
                types,
            }],
            warnings: vec![],
        }
    }

    #[test]
    fn empty_graph_emits_nothing_meaningful() {
        let g = SchemaGraph {
            version: SCHEMA_GRAPH_VERSION.into(),
            database: "x".into(),
            schemas: vec![],
            warnings: vec![],
        };
        assert_eq!(baseline_ddl(&g), "\n");
    }

    #[test]
    fn dbo_schema_does_not_emit_create_schema() {
        let t = Table {
            name: "T".into(),
            columns: vec![col("id", "[int]", false)],
            primary_key: None,
            unique_constraints: vec![],
            check_constraints: vec![],
            foreign_keys: vec![],
            indexes: vec![],
            triggers: vec![],
        };
        let sql = baseline_ddl(&graph_with("dbo", vec![t], vec![]));
        assert!(!sql.contains("CREATE SCHEMA"));
        assert!(sql.contains("CREATE TABLE [dbo].[T]"));
    }

    #[test]
    fn non_dbo_schema_emitted_idempotently() {
        let t = Table {
            name: "T".into(),
            columns: vec![col("id", "[int]", false)],
            primary_key: None,
            unique_constraints: vec![],
            check_constraints: vec![],
            foreign_keys: vec![],
            indexes: vec![],
            triggers: vec![],
        };
        let sql = baseline_ddl(&graph_with("sales", vec![t], vec![]));
        assert!(sql.contains("IF SCHEMA_ID(N'sales') IS NULL"));
        assert!(sql.contains("CREATE SCHEMA [sales]"));
    }

    #[test]
    fn table_with_pk_and_index_and_fk_round_trips() {
        let t = Table {
            name: "Customer".into(),
            columns: vec![
                Column {
                    name: "Id".into(),
                    type_name: "[int]".into(),
                    nullable: false,
                    identity: true,
                    is_uddt: false,
                    base_type: None,
                    default: None,
                    computed: None,
                },
                col("Email", "[nvarchar](255)", false),
            ],
            primary_key: Some(KeyConstraint {
                name: "PK_Customer".into(),
                columns: vec!["Id".into()],
                clustered: true,
            }),
            unique_constraints: vec![KeyConstraint {
                name: "UQ_Customer_Email".into(),
                columns: vec!["Email".into()],
                clustered: false,
            }],
            check_constraints: vec![CheckConstraint {
                name: "CK_Customer_Email".into(),
                expression: "([Email] LIKE '%@%')".into(),
            }],
            foreign_keys: vec![],
            indexes: vec![Index {
                name: "IX_Customer_Email".into(),
                columns: vec!["Email".into()],
                included_columns: vec![],
                is_unique: false,
                is_clustered: false,
                filter: Some("([Email] IS NOT NULL)".into()),
            }],
            triggers: vec![],
        };
        let order = Table {
            name: "Order".into(),
            columns: vec![col("Id", "[int]", false), col("CustomerId", "[int]", false)],
            primary_key: None,
            unique_constraints: vec![],
            check_constraints: vec![],
            foreign_keys: vec![ForeignKey {
                name: "FK_Order_Customer".into(),
                columns: vec!["CustomerId".into()],
                referenced_schema: "dbo".into(),
                referenced_table: "Customer".into(),
                referenced_columns: vec!["Id".into()],
                on_delete: "CASCADE".into(),
                on_update: "NO_ACTION".into(),
            }],
            indexes: vec![],
            triggers: vec![],
        };
        let sql = baseline_ddl(&graph_with("dbo", vec![order, t], vec![]));
        // Tables sorted alphabetically.
        let cust_pos = sql.find("CREATE TABLE [dbo].[Customer]").unwrap();
        let ord_pos = sql.find("CREATE TABLE [dbo].[Order]").unwrap();
        assert!(cust_pos < ord_pos);
        // FK comes after both CREATE TABLEs.
        let fk_pos = sql.find("ADD CONSTRAINT [FK_Order_Customer]").unwrap();
        assert!(fk_pos > ord_pos);
        // PK / UQ inline.
        assert!(sql.contains("CONSTRAINT [PK_Customer] PRIMARY KEY CLUSTERED ([Id])"));
        assert!(sql.contains("CONSTRAINT [UQ_Customer_Email] UNIQUE NONCLUSTERED ([Email])"));
        // Index with WHERE filter.
        assert!(sql.contains("CREATE NONCLUSTERED INDEX [IX_Customer_Email]"));
        assert!(sql.contains("WHERE ([Email] IS NOT NULL)"));
        // FK actions.
        assert!(sql.contains("ON DELETE CASCADE"));
        assert!(!sql.contains("ON UPDATE"));
        // Identity.
        assert!(sql.contains("IDENTITY(1,1)"));
    }

    #[test]
    fn output_is_deterministic() {
        let mk = || {
            let t = Table {
                name: "T".into(),
                columns: vec![col("a", "[int]", false), col("b", "[int]", true)],
                primary_key: None,
                unique_constraints: vec![],
                check_constraints: vec![],
                foreign_keys: vec![],
                indexes: vec![],
                triggers: vec![],
            };
            graph_with("dbo", vec![t], vec![])
        };
        assert_eq!(baseline_ddl(&mk()), baseline_ddl(&mk()));
    }
}
