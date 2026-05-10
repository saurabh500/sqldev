// sqldev M0.2 spike: schema-graph differ.
//
// Consumes two schema-graph JSON files (v0.1) produced by spike-introspect
// and emits T-SQL that, when applied to the `old` database, transitions it
// to the `new` schema.
//
// Goals (M0 gate):
//   * "Common path" correctness: ADD/DROP TABLE, ADD/DROP/ALTER COLUMN,
//     ADD/DROP CHECK, ADD/DROP FK, ADD/DROP INDEX, ADD/DROP UQ, change PK.
//   * Deterministic, line-stable output ordered safely (drops before adds
//     where dependencies require it).
//
// Non-goals: rename detection, online ALTER, data movement, computed-column
// expression diffing, view/proc/fn diff (these come in M2).

use anyhow::{Context, Result};
use clap::Parser;
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(version, about = "sqldev diff spike (schema-graph -> T-SQL)")]
struct Args {
    /// Old schema-graph JSON (the database's current state).
    #[arg(long)]
    old: PathBuf,
    /// New schema-graph JSON (the target state).
    #[arg(long)]
    new: PathBuf,
}

// --- Schema-graph types (read-only mirror of spike-introspect output). ---

#[derive(Deserialize)]
struct Graph {
    #[allow(dead_code)]
    version: String,
    schemas: Vec<Schema>,
}

#[derive(Deserialize)]
struct Schema {
    name: String,
    #[serde(default)]
    tables: Vec<Table>,
    #[serde(default)]
    types: Vec<Udt>,
}

#[derive(Deserialize, Clone)]
struct Table {
    name: String,
    columns: Vec<Column>,
    #[serde(default)]
    primary_key: Option<KeyConstraint>,
    #[serde(default)]
    unique_constraints: Vec<KeyConstraint>,
    #[serde(default)]
    check_constraints: Vec<CheckConstraint>,
    #[serde(default)]
    foreign_keys: Vec<ForeignKey>,
    #[serde(default)]
    indexes: Vec<Index>,
}

#[derive(Deserialize, Clone, PartialEq, Eq)]
struct Column {
    name: String,
    type_name: String,
    nullable: bool,
    #[serde(default)]
    identity: bool,
    #[serde(default)]
    is_uddt: bool,
    #[serde(default)]
    base_type: Option<String>,
    #[serde(default)]
    default: Option<String>,
    #[serde(default)]
    computed: Option<String>,
}

#[derive(Deserialize, Clone, PartialEq, Eq)]
struct KeyConstraint {
    name: String,
    columns: Vec<String>,
    clustered: bool,
}

#[derive(Deserialize, Clone, PartialEq, Eq)]
struct CheckConstraint {
    name: String,
    expression: String,
}

#[derive(Deserialize, Clone, PartialEq, Eq)]
struct ForeignKey {
    name: String,
    columns: Vec<String>,
    referenced_schema: String,
    referenced_table: String,
    referenced_columns: Vec<String>,
    on_delete: String,
    on_update: String,
}

#[derive(Deserialize, Clone, PartialEq, Eq)]
struct Index {
    name: String,
    columns: Vec<String>,
    #[serde(default)]
    included_columns: Vec<String>,
    is_unique: bool,
    is_clustered: bool,
    #[serde(default)]
    filter: Option<String>,
}

#[derive(Deserialize, Clone)]
#[allow(dead_code)]
struct Udt {
    name: String,
    base_type: String,
    nullable: bool,
}

// --- Indexed view of a graph for fast lookup. ---

struct Indexed<'g> {
    /// (schema, table) -> Table
    tables: BTreeMap<(String, String), &'g Table>,
}

impl<'g> Indexed<'g> {
    fn from(graph: &'g Graph) -> Self {
        let mut tables = BTreeMap::new();
        for s in &graph.schemas {
            for t in &s.tables {
                tables.insert((s.name.clone(), t.name.clone()), t);
            }
        }
        Indexed { tables }
    }
}

fn main() -> Result<()> {
    let args = Args::parse();
    let old: Graph =
        serde_json::from_str(&fs::read_to_string(&args.old).context("reading --old")?)?;
    let new: Graph =
        serde_json::from_str(&fs::read_to_string(&args.new).context("reading --new")?)?;

    let old_idx = Indexed::from(&old);
    let new_idx = Indexed::from(&new);

    let mut out = Vec::<String>::new();

    // ---- Phase 1: drops that block other operations. ----
    for (key, old_t) in &old_idx.tables {
        let new_t = new_idx.tables.get(key);
        for fk in &old_t.foreign_keys {
            let unchanged = new_t
                .map(|n| n.foreign_keys.iter().any(|f| f.name == fk.name && f == fk))
                .unwrap_or(false);
            if !unchanged {
                out.push(format!(
                    "ALTER TABLE [{}].[{}] DROP CONSTRAINT [{}];",
                    key.0, key.1, fk.name
                ));
            }
        }
    }
    for (key, old_t) in &old_idx.tables {
        let new_t = new_idx.tables.get(key);
        for cc in &old_t.check_constraints {
            let unchanged = new_t
                .map(|n| n.check_constraints.iter().any(|c| c.name == cc.name && c == cc))
                .unwrap_or(false);
            if !unchanged {
                out.push(format!(
                    "ALTER TABLE [{}].[{}] DROP CONSTRAINT [{}];",
                    key.0, key.1, cc.name
                ));
            }
        }
    }
    for (key, old_t) in &old_idx.tables {
        let new_t = new_idx.tables.get(key);
        for ix in &old_t.indexes {
            let unchanged = new_t
                .map(|n| n.indexes.iter().any(|i| i.name == ix.name && i == ix))
                .unwrap_or(false);
            if !unchanged {
                out.push(format!(
                    "DROP INDEX [{}] ON [{}].[{}];",
                    ix.name, key.0, key.1
                ));
            }
        }
    }
    for (key, old_t) in &old_idx.tables {
        let new_t = new_idx.tables.get(key);
        for uq in &old_t.unique_constraints {
            let unchanged = new_t
                .map(|n| n.unique_constraints.iter().any(|u| u.name == uq.name && u == uq))
                .unwrap_or(false);
            if !unchanged {
                out.push(format!(
                    "ALTER TABLE [{}].[{}] DROP CONSTRAINT [{}];",
                    key.0, key.1, uq.name
                ));
            }
        }
    }

    // ---- Phase 2: drop tables that no longer exist. ----
    for key in old_idx.tables.keys() {
        if !new_idx.tables.contains_key(key) {
            out.push(format!("DROP TABLE [{}].[{}];", key.0, key.1));
        }
    }

    // ---- Phase 3: per-table column-level changes. ----
    for (key, new_t) in &new_idx.tables {
        let Some(old_t) = old_idx.tables.get(key) else {
            continue;
        };
        let old_cols: BTreeMap<&str, &Column> =
            old_t.columns.iter().map(|c| (c.name.as_str(), c)).collect();
        let new_cols: BTreeMap<&str, &Column> =
            new_t.columns.iter().map(|c| (c.name.as_str(), c)).collect();

        for cname in old_cols.keys() {
            if !new_cols.contains_key(cname) {
                out.push(format!(
                    "ALTER TABLE [{}].[{}] DROP COLUMN [{}];",
                    key.0, key.1, cname
                ));
            }
        }
        for (cname, col) in &new_cols {
            if !old_cols.contains_key(cname) {
                out.push(add_column_sql(&key.0, &key.1, col));
            }
        }
        for (cname, new_col) in &new_cols {
            let Some(old_col) = old_cols.get(cname) else {
                continue;
            };
            if new_col != old_col {
                out.extend(alter_column_sql(&key.0, &key.1, old_col, new_col));
            }
        }
    }

    // ---- Phase 4: create tables that didn't exist. ----
    for (key, new_t) in &new_idx.tables {
        if !old_idx.tables.contains_key(key) {
            out.push(create_table_sql(&key.0, &key.1, new_t));
        }
    }

    // ---- Phase 5: re-add unique constraints. ----
    for (key, new_t) in &new_idx.tables {
        let old_t = old_idx.tables.get(key);
        for uq in &new_t.unique_constraints {
            let existed = old_t
                .map(|o| o.unique_constraints.iter().any(|u| u.name == uq.name && u == uq))
                .unwrap_or(false);
            if !existed {
                out.push(format!(
                    "ALTER TABLE [{}].[{}] ADD CONSTRAINT [{}] UNIQUE {} ({});",
                    key.0,
                    key.1,
                    uq.name,
                    if uq.clustered {
                        "CLUSTERED"
                    } else {
                        "NONCLUSTERED"
                    },
                    uq.columns
                        .iter()
                        .map(|c| format!("[{}]", c))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
        }
    }

    // ---- Phase 6: re-add CHECK constraints. ----
    for (key, new_t) in &new_idx.tables {
        let old_t = old_idx.tables.get(key);
        for cc in &new_t.check_constraints {
            let existed = old_t
                .map(|o| o.check_constraints.iter().any(|c| c.name == cc.name && c == cc))
                .unwrap_or(false);
            if !existed {
                out.push(format!(
                    "ALTER TABLE [{}].[{}] ADD CONSTRAINT [{}] CHECK {};",
                    key.0, key.1, cc.name, cc.expression
                ));
            }
        }
    }

    // ---- Phase 7: re-add indexes. ----
    for (key, new_t) in &new_idx.tables {
        let old_t = old_idx.tables.get(key);
        for ix in &new_t.indexes {
            let existed = old_t
                .map(|o| o.indexes.iter().any(|i| i.name == ix.name && i == ix))
                .unwrap_or(false);
            if !existed {
                out.push(create_index_sql(&key.0, &key.1, ix));
            }
        }
    }

    // ---- Phase 8: re-add foreign keys. ----
    for (key, new_t) in &new_idx.tables {
        let old_t = old_idx.tables.get(key);
        for fk in &new_t.foreign_keys {
            let existed = old_t
                .map(|o| o.foreign_keys.iter().any(|f| f.name == fk.name && f == fk))
                .unwrap_or(false);
            if !existed {
                out.push(create_fk_sql(&key.0, &key.1, fk));
            }
        }
    }

    // Surface coverage gaps.
    let mut warnings = BTreeSet::new();
    for s in old.schemas.iter().chain(new.schemas.iter()) {
        for t in &s.tables {
            for c in &t.columns {
                if c.computed.is_some() {
                    warnings.insert(format!(
                        "computed column not diffed: {}.{}.{}",
                        s.name, t.name, c.name
                    ));
                }
            }
        }
        if !s.types.is_empty() {
            warnings.insert(format!(
                "user-defined types not diffed in schema {}",
                s.name
            ));
        }
    }
    for w in &warnings {
        eprintln!("[diff] note: {}", w);
    }

    if out.is_empty() {
        println!("-- no changes");
    } else {
        for stmt in out {
            println!("{}", stmt);
        }
    }
    Ok(())
}

// --- SQL emitters ---------------------------------------------------------

fn col_type(c: &Column) -> String {
    if c.is_uddt {
        // UDDT: assume dbo for the spike. M1 will plumb owning schema.
        format!("[dbo].[{}]", c.type_name)
    } else {
        c.type_name.clone()
    }
}

fn col_definition(c: &Column) -> String {
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
        parts.push(format!("DEFAULT ({})", d));
    }
    parts.join(" ")
}

fn add_column_sql(schema: &str, table: &str, c: &Column) -> String {
    format!(
        "ALTER TABLE [{}].[{}] ADD {};",
        schema,
        table,
        col_definition(c)
    )
}

fn alter_column_sql(schema: &str, table: &str, old: &Column, new: &Column) -> Vec<String> {
    let mut stmts = vec![];
    if old.identity != new.identity || old.computed != new.computed {
        eprintln!(
            "[diff] warning: column [{}].[{}].[{}] changed identity/computed; manual migration required",
            schema, table, new.name
        );
        return stmts;
    }
    let null_clause = if new.nullable { "NULL" } else { "NOT NULL" };
    if old.type_name != new.type_name
        || old.is_uddt != new.is_uddt
        || old.nullable != new.nullable
    {
        stmts.push(format!(
            "ALTER TABLE [{}].[{}] ALTER COLUMN [{}] {} {};",
            schema,
            table,
            new.name,
            col_type(new),
            null_clause
        ));
    }
    if old.default != new.default {
        if old.default.is_some() {
            eprintln!(
                "[diff] warning: dropping default on [{}].[{}].[{}] needs constraint name (not in schema graph yet)",
                schema, table, new.name
            );
        }
        if let Some(d) = &new.default {
            stmts.push(format!(
                "ALTER TABLE [{}].[{}] ADD DEFAULT ({}) FOR [{}];",
                schema, table, d, new.name
            ));
        }
    }
    stmts
}

fn create_table_sql(schema: &str, name: &str, t: &Table) -> String {
    let mut lines: Vec<String> = t
        .columns
        .iter()
        .map(|c| {
            if let Some(expr) = &c.computed {
                format!("    [{}] AS {}", c.name, expr)
            } else {
                format!("    {}", col_definition(c))
            }
        })
        .collect();
    if let Some(pk) = &t.primary_key {
        lines.push(format!(
            "    CONSTRAINT [{}] PRIMARY KEY {} ({})",
            pk.name,
            if pk.clustered {
                "CLUSTERED"
            } else {
                "NONCLUSTERED"
            },
            pk.columns
                .iter()
                .map(|c| format!("[{}]", c))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    for uq in &t.unique_constraints {
        lines.push(format!(
            "    CONSTRAINT [{}] UNIQUE {} ({})",
            uq.name,
            if uq.clustered {
                "CLUSTERED"
            } else {
                "NONCLUSTERED"
            },
            uq.columns
                .iter()
                .map(|c| format!("[{}]", c))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    for cc in &t.check_constraints {
        lines.push(format!("    CONSTRAINT [{}] CHECK {}", cc.name, cc.expression));
    }
    format!(
        "CREATE TABLE [{}].[{}] (\n{}\n);",
        schema,
        name,
        lines.join(",\n")
    )
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
        .map(|c| format!("[{}]", c))
        .collect::<Vec<_>>()
        .join(", ");
    let mut sql = format!(
        "CREATE {}{}INDEX [{}] ON [{}].[{}] ({})",
        unique, clustered, ix.name, schema, table, cols
    );
    if !ix.included_columns.is_empty() {
        let inc = ix
            .included_columns
            .iter()
            .map(|c| format!("[{}]", c))
            .collect::<Vec<_>>()
            .join(", ");
        sql.push_str(&format!(" INCLUDE ({})", inc));
    }
    if let Some(f) = &ix.filter {
        sql.push_str(&format!(" WHERE {}", f));
    }
    sql.push(';');
    sql
}

fn create_fk_sql(schema: &str, table: &str, fk: &ForeignKey) -> String {
    let cols = fk
        .columns
        .iter()
        .map(|c| format!("[{}]", c))
        .collect::<Vec<_>>()
        .join(", ");
    let refcols = fk
        .referenced_columns
        .iter()
        .map(|c| format!("[{}]", c))
        .collect::<Vec<_>>()
        .join(", ");
    let mut sql = format!(
        "ALTER TABLE [{}].[{}] ADD CONSTRAINT [{}] FOREIGN KEY ({}) REFERENCES [{}].[{}] ({})",
        schema, table, fk.name, cols, fk.referenced_schema, fk.referenced_table, refcols
    );
    if fk.on_delete != "NO_ACTION" && !fk.on_delete.is_empty() {
        sql.push_str(&format!(" ON DELETE {}", fk.on_delete.replace('_', " ")));
    }
    if fk.on_update != "NO_ACTION" && !fk.on_update.is_empty() {
        sql.push_str(&format!(" ON UPDATE {}", fk.on_update.replace('_', " ")));
    }
    sql.push(';');
    sql
}
