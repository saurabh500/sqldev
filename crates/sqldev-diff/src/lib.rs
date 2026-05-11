//! Schema-graph differ: produces deterministic T-SQL that transitions
//! `old` -> `new`.
//!
//! Productionized from `spike-diff/`, retargeted onto the canonical
//! [`sqldev_core`] schema-graph types (no private mirror) and extended
//! beyond the spike's table-only scope with:
//!
//! * `CREATE SCHEMA` for new non-`dbo` schemas
//! * Body-level diff for views, stored procedures, functions, triggers
//!   (drop + create on definition change; rename is not detected)
//!
//! The differ does **not** emit data movement, online ALTER, rename
//! detection, sequence DDL, or named-default-constraint drops yet —
//! those are tracked as follow-up issues.
//!
//! # Output ordering
//!
//! Statements are emitted in dependency-safe phases:
//!
//! 1. drop FKs that change/disappear
//! 2. drop CHECKs that change/disappear
//! 3. drop indexes that change/disappear
//! 4. drop UQs that change/disappear
//! 5. drop triggers that change/disappear
//! 6. drop views/procs/functions whose body changed (so we can recreate)
//! 7. drop tables that disappeared
//! 8. per-surviving-table column add/drop/alter
//! 9. create new schemas
//! 10. create new tables (with inline PK / UQ / CHECK)
//! 11. create UQs added to existing tables
//! 12. create CHECKs added to existing tables
//! 13. create / re-create indexes
//! 14. create / re-create triggers
//! 15. create FKs (last — after every referenced table exists)
//! 16. create / re-create views, procedures, functions

#![forbid(unsafe_code)]

use sqldev_core::{
    Column, ForeignKey, Index, KeyConstraint, Routine, SchemaGraph, SchemaNode, Table, Trigger,
    View,
};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

/// Result of comparing two schema graphs.
#[derive(Debug, Clone, Default)]
pub struct DiffResult {
    /// T-SQL statements to apply, in order.
    pub statements: Vec<String>,
    /// Non-fatal coverage gaps (e.g. unsupported object kinds present in
    /// either graph). Callers typically print these to stderr.
    pub warnings: Vec<String>,
}

/// Compare two graphs and return the migration plan.
#[must_use]
pub fn diff(old: &SchemaGraph, new: &SchemaGraph) -> DiffResult {
    let old_tables = index_tables(old);
    let new_tables = index_tables(new);
    let old_schemas: BTreeMap<&str, &SchemaNode> =
        old.schemas.iter().map(|s| (s.name.as_str(), s)).collect();
    let new_schemas: BTreeMap<&str, &SchemaNode> =
        new.schemas.iter().map(|s| (s.name.as_str(), s)).collect();

    let mut out: Vec<String> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();

    // ---- Phase 1-4: drops that block other operations ----
    drop_fks(&old_tables, &new_tables, &mut out);
    drop_checks(&old_tables, &new_tables, &mut out);
    drop_indexes(&old_tables, &new_tables, &mut out);
    drop_uqs(&old_tables, &new_tables, &mut out);

    // ---- Phase 5: drop triggers whose definition or table is gone/changed.
    drop_triggers(&old_tables, &new_tables, &mut out);

    // ---- Phase 6: drop views / procs / fns whose body changed or vanished.
    drop_routines(
        &old_schemas,
        &new_schemas,
        |s| &s.views,
        |name, schema| format!("DROP VIEW [{schema}].[{name}];"),
        view_changed,
        &mut out,
    );
    drop_routines(
        &old_schemas,
        &new_schemas,
        |s| &s.procedures,
        |name, schema| format!("DROP PROCEDURE [{schema}].[{name}];"),
        routine_changed,
        &mut out,
    );
    drop_routines(
        &old_schemas,
        &new_schemas,
        |s| &s.functions,
        |name, schema| format!("DROP FUNCTION [{schema}].[{name}];"),
        routine_changed,
        &mut out,
    );

    // ---- Phase 7: drop tables that no longer exist.
    for key in old_tables.keys() {
        if !new_tables.contains_key(key) {
            out.push(format!("DROP TABLE [{}].[{}];", key.0, key.1));
        }
    }

    // ---- Phase 8: per-surviving-table column changes.
    for (key, new_t) in &new_tables {
        let Some(old_t) = old_tables.get(key) else {
            continue;
        };
        diff_columns(&key.0, &key.1, old_t, new_t, &mut out, &mut warnings);
    }

    // ---- Phase 9: create new schemas (skip dbo, always present).
    for name in new_schemas.keys() {
        if *name == "dbo" || old_schemas.contains_key(name) {
            continue;
        }
        out.push(format!(
            "IF SCHEMA_ID(N'{name}') IS NULL EXEC(N'CREATE SCHEMA [{name}]');"
        ));
    }

    // ---- Phase 10: create new tables.
    for (key, new_t) in &new_tables {
        if !old_tables.contains_key(key) {
            out.push(create_table_sql(&key.0, &key.1, new_t));
        }
    }

    // ---- Phase 11-13: re-add UQ / CHECK / indexes that the drop phase removed.
    add_uqs(&old_tables, &new_tables, &mut out);
    add_checks(&old_tables, &new_tables, &mut out);
    add_indexes(&old_tables, &new_tables, &mut out);

    // ---- Phase 14: re-add triggers.
    add_triggers(&old_tables, &new_tables, &mut out);

    // ---- Phase 15: re-add FKs (last — after all tables exist).
    add_fks(&old_tables, &new_tables, &mut out);

    // ---- Phase 16: re-create views / procs / fns whose body changed/added.
    create_routines(
        &old_schemas,
        &new_schemas,
        |s| &s.views,
        view_definition,
        &mut out,
        &mut warnings,
        "view",
    );
    create_routines(
        &old_schemas,
        &new_schemas,
        |s| &s.procedures,
        routine_definition,
        &mut out,
        &mut warnings,
        "procedure",
    );
    create_routines(
        &old_schemas,
        &new_schemas,
        |s| &s.functions,
        routine_definition,
        &mut out,
        &mut warnings,
        "function",
    );

    // Collect coverage warnings.
    collect_warnings(old, new, &mut warnings);

    DiffResult {
        statements: out,
        warnings,
    }
}

// ---------- indexed views ------------------------------------------------

type TableIndex<'g> = BTreeMap<(String, String), &'g Table>;

fn index_tables(g: &SchemaGraph) -> TableIndex<'_> {
    let mut out = BTreeMap::new();
    for s in &g.schemas {
        for t in &s.tables {
            out.insert((s.name.clone(), t.name.clone()), t);
        }
    }
    out
}

// ---------- drop helpers -------------------------------------------------

fn drop_fks(old: &TableIndex, new: &TableIndex, out: &mut Vec<String>) {
    for (key, old_t) in old {
        let new_t = new.get(key);
        for fk in &old_t.foreign_keys {
            if !same_named(new_t.map(|t| &t.foreign_keys[..]), &fk.name, fk) {
                out.push(format!(
                    "ALTER TABLE [{}].[{}] DROP CONSTRAINT [{}];",
                    key.0, key.1, fk.name
                ));
            }
        }
    }
}

fn drop_checks(old: &TableIndex, new: &TableIndex, out: &mut Vec<String>) {
    for (key, old_t) in old {
        let new_t = new.get(key);
        for cc in &old_t.check_constraints {
            if !same_named(new_t.map(|t| &t.check_constraints[..]), &cc.name, cc) {
                out.push(format!(
                    "ALTER TABLE [{}].[{}] DROP CONSTRAINT [{}];",
                    key.0, key.1, cc.name
                ));
            }
        }
    }
}

fn drop_indexes(old: &TableIndex, new: &TableIndex, out: &mut Vec<String>) {
    for (key, old_t) in old {
        let new_t = new.get(key);
        for ix in &old_t.indexes {
            if !same_named(new_t.map(|t| &t.indexes[..]), &ix.name, ix) {
                out.push(format!(
                    "DROP INDEX [{}] ON [{}].[{}];",
                    ix.name, key.0, key.1
                ));
            }
        }
    }
}

fn drop_uqs(old: &TableIndex, new: &TableIndex, out: &mut Vec<String>) {
    for (key, old_t) in old {
        let new_t = new.get(key);
        for uq in &old_t.unique_constraints {
            if !same_named(new_t.map(|t| &t.unique_constraints[..]), &uq.name, uq) {
                out.push(format!(
                    "ALTER TABLE [{}].[{}] DROP CONSTRAINT [{}];",
                    key.0, key.1, uq.name
                ));
            }
        }
    }
}

fn drop_triggers(old: &TableIndex, new: &TableIndex, out: &mut Vec<String>) {
    for (key, old_t) in old {
        let new_t = new.get(key);
        for tr in &old_t.triggers {
            if !same_named(new_t.map(|t| &t.triggers[..]), &tr.name, tr) {
                out.push(format!("DROP TRIGGER [{}].[{}];", key.0, tr.name));
            }
        }
    }
}

// ---------- add helpers --------------------------------------------------

fn add_uqs(old: &TableIndex, new: &TableIndex, out: &mut Vec<String>) {
    for (key, new_t) in new {
        let old_t = old.get(key);
        for uq in &new_t.unique_constraints {
            if same_named(old_t.map(|t| &t.unique_constraints[..]), &uq.name, uq) {
                continue;
            }
            // Skip if it was emitted inline by CREATE TABLE in phase 10.
            if old_t.is_none() {
                continue;
            }
            out.push(uq_sql(&key.0, &key.1, uq));
        }
    }
}

fn add_checks(old: &TableIndex, new: &TableIndex, out: &mut Vec<String>) {
    for (key, new_t) in new {
        let old_t = old.get(key);
        for cc in &new_t.check_constraints {
            if same_named(old_t.map(|t| &t.check_constraints[..]), &cc.name, cc) {
                continue;
            }
            if old_t.is_none() {
                continue;
            }
            out.push(format!(
                "ALTER TABLE [{}].[{}] ADD CONSTRAINT [{}] CHECK {};",
                key.0, key.1, cc.name, cc.expression
            ));
        }
    }
}

fn add_indexes(old: &TableIndex, new: &TableIndex, out: &mut Vec<String>) {
    for (key, new_t) in new {
        let old_t = old.get(key);
        for ix in &new_t.indexes {
            if same_named(old_t.map(|t| &t.indexes[..]), &ix.name, ix) {
                continue;
            }
            // Indexes are NOT emitted by CREATE TABLE — always emit, even
            // for new tables.
            out.push(create_index_sql(&key.0, &key.1, ix));
        }
    }
}

fn add_triggers(old: &TableIndex, new: &TableIndex, out: &mut Vec<String>) {
    for (key, new_t) in new {
        let old_t = old.get(key);
        for tr in &new_t.triggers {
            if same_named(old_t.map(|t| &t.triggers[..]), &tr.name, tr) {
                continue;
            }
            if let Some(def) = &tr.definition {
                out.push(format!("{};", def.trim_end_matches(';').trim_end()));
            } else {
                out.push(format!(
                    "-- WARNING: trigger [{}].[{}] has no captured definition",
                    key.0, tr.name
                ));
            }
        }
    }
}

fn add_fks(old: &TableIndex, new: &TableIndex, out: &mut Vec<String>) {
    for (key, new_t) in new {
        let old_t = old.get(key);
        for fk in &new_t.foreign_keys {
            if same_named(old_t.map(|t| &t.foreign_keys[..]), &fk.name, fk) {
                continue;
            }
            out.push(create_fk_sql(&key.0, &key.1, fk));
        }
    }
}

// ---------- routine (view/proc/fn) helpers --------------------------------

fn drop_routines<R, F, G, C>(
    old_schemas: &BTreeMap<&str, &SchemaNode>,
    new_schemas: &BTreeMap<&str, &SchemaNode>,
    pick: F,
    drop_sql: G,
    changed: C,
    out: &mut Vec<String>,
) where
    F: Fn(&SchemaNode) -> &Vec<R>,
    G: Fn(&str, &str) -> String,
    C: Fn(&R, &R) -> bool,
    R: HasName,
{
    for (sname, old_s) in old_schemas {
        let new_routines: Vec<&R> = new_schemas
            .get(sname)
            .map(|s| pick(s).iter().collect())
            .unwrap_or_default();
        for r in pick(old_s) {
            let still_present = new_routines.iter().find(|n| n.name() == r.name());
            match still_present {
                None => out.push(drop_sql(r.name(), sname)),
                Some(n) => {
                    if changed(r, n) {
                        out.push(drop_sql(r.name(), sname));
                    }
                }
            }
        }
    }
}

fn create_routines<R, F, D>(
    old_schemas: &BTreeMap<&str, &SchemaNode>,
    new_schemas: &BTreeMap<&str, &SchemaNode>,
    pick: F,
    def: D,
    out: &mut Vec<String>,
    warnings: &mut Vec<String>,
    kind_label: &str,
) where
    F: Fn(&SchemaNode) -> &Vec<R>,
    D: Fn(&R) -> Option<&str>,
    R: HasName,
{
    for (sname, new_s) in new_schemas {
        let old_routines: Vec<&R> = old_schemas
            .get(sname)
            .map(|s| pick(s).iter().collect())
            .unwrap_or_default();
        for r in pick(new_s) {
            let prev = old_routines.iter().find(|o| o.name() == r.name()).copied();
            let needs_create = match prev {
                None => true,
                Some(p) => match (def(p), def(r)) {
                    (Some(a), Some(b)) => a.trim() != b.trim(),
                    _ => true,
                },
            };
            if !needs_create {
                continue;
            }
            if let Some(d) = def(r) {
                out.push(format!("{};", d.trim_end_matches(';').trim_end()));
            } else {
                warnings.push(format!(
                    "{kind_label} [{sname}].[{name}] has no captured definition; skipped",
                    name = r.name()
                ));
            }
        }
    }
}

trait HasName {
    fn name(&self) -> &str;
}
impl HasName for View {
    fn name(&self) -> &str {
        &self.name
    }
}
impl HasName for Routine {
    fn name(&self) -> &str {
        &self.name
    }
}
impl HasName for Trigger {
    fn name(&self) -> &str {
        &self.name
    }
}

fn view_changed(a: &View, b: &View) -> bool {
    normalize(a.definition.as_deref()) != normalize(b.definition.as_deref())
}
fn routine_changed(a: &Routine, b: &Routine) -> bool {
    a.kind != b.kind || normalize(a.definition.as_deref()) != normalize(b.definition.as_deref())
}
fn view_definition(v: &View) -> Option<&str> {
    v.definition.as_deref()
}
fn routine_definition(r: &Routine) -> Option<&str> {
    r.definition.as_deref()
}
fn normalize(s: Option<&str>) -> String {
    s.map(|t| t.trim().to_string()).unwrap_or_default()
}

// ---------- column diff --------------------------------------------------

fn diff_columns(
    schema: &str,
    table: &str,
    old_t: &Table,
    new_t: &Table,
    out: &mut Vec<String>,
    warnings: &mut Vec<String>,
) {
    let old_cols: BTreeMap<&str, &Column> =
        old_t.columns.iter().map(|c| (c.name.as_str(), c)).collect();
    let new_cols: BTreeMap<&str, &Column> =
        new_t.columns.iter().map(|c| (c.name.as_str(), c)).collect();

    for cname in old_cols.keys() {
        if !new_cols.contains_key(cname) {
            out.push(format!(
                "ALTER TABLE [{schema}].[{table}] DROP COLUMN [{cname}];"
            ));
        }
    }
    for (cname, col) in &new_cols {
        if !old_cols.contains_key(cname) {
            out.push(add_column_sql(schema, table, col));
        }
    }
    for (cname, new_col) in &new_cols {
        let Some(old_col) = old_cols.get(cname) else {
            continue;
        };
        if new_col != old_col {
            out.extend(alter_column_sql(schema, table, old_col, new_col, warnings));
        }
    }
}

// ---------- SQL emitters -------------------------------------------------

fn col_type(c: &Column) -> String {
    if c.is_uddt {
        let schema = c.udt_schema.as_deref().unwrap_or("dbo");
        format!("[{schema}].[{}]", c.type_name)
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
        parts.push(format!("DEFAULT ({d})"));
    }
    parts.join(" ")
}

fn add_column_sql(schema: &str, table: &str, c: &Column) -> String {
    format!(
        "ALTER TABLE [{schema}].[{table}] ADD {};",
        col_definition(c)
    )
}

fn alter_column_sql(
    schema: &str,
    table: &str,
    old: &Column,
    new: &Column,
    warnings: &mut Vec<String>,
) -> Vec<String> {
    let mut stmts = vec![];
    if old.identity != new.identity || old.computed != new.computed {
        warnings.push(format!(
            "column [{schema}].[{table}].[{}] changed identity/computed; manual migration required",
            new.name
        ));
        return stmts;
    }
    let null_clause = if new.nullable { "NULL" } else { "NOT NULL" };
    if old.type_name != new.type_name || old.is_uddt != new.is_uddt || old.nullable != new.nullable
    {
        stmts.push(format!(
            "ALTER TABLE [{schema}].[{table}] ALTER COLUMN [{}] {} {null_clause};",
            new.name,
            col_type(new),
        ));
    }
    if old.default != new.default {
        if old.default.is_some() {
            warnings.push(format!(
                "dropping default on [{schema}].[{table}].[{}] needs constraint name (not in schema graph yet)",
                new.name
            ));
        }
        if let Some(d) = &new.default {
            stmts.push(format!(
                "ALTER TABLE [{schema}].[{table}] ADD DEFAULT ({d}) FOR [{}];",
                new.name
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
                format!("    [{}] AS {expr}", c.name)
            } else {
                format!("    {}", col_definition(c))
            }
        })
        .collect();
    if let Some(pk) = &t.primary_key {
        lines.push(format!(
            "    CONSTRAINT [{}] PRIMARY KEY {} ({})",
            pk.name,
            clustered_kw(pk.clustered),
            cols_csv(&pk.columns)
        ));
    }
    for uq in &t.unique_constraints {
        lines.push(format!(
            "    CONSTRAINT [{}] UNIQUE {} ({})",
            uq.name,
            clustered_kw(uq.clustered),
            cols_csv(&uq.columns)
        ));
    }
    for cc in &t.check_constraints {
        lines.push(format!(
            "    CONSTRAINT [{}] CHECK {}",
            cc.name, cc.expression
        ));
    }
    format!(
        "CREATE TABLE [{schema}].[{name}] (\n{}\n);",
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
    let mut sql = format!(
        "CREATE {unique}{clustered}INDEX [{}] ON [{schema}].[{table}] ({})",
        ix.name,
        cols_csv(&ix.columns)
    );
    if !ix.included_columns.is_empty() {
        write!(sql, " INCLUDE ({})", cols_csv(&ix.included_columns)).unwrap();
    }
    if let Some(f) = &ix.filter {
        write!(sql, " WHERE {f}").unwrap();
    }
    sql.push(';');
    sql
}

fn create_fk_sql(schema: &str, table: &str, fk: &ForeignKey) -> String {
    let mut sql = format!(
        "ALTER TABLE [{schema}].[{table}] ADD CONSTRAINT [{}] FOREIGN KEY ({}) REFERENCES [{}].[{}] ({})",
        fk.name,
        cols_csv(&fk.columns),
        fk.referenced_schema,
        fk.referenced_table,
        cols_csv(&fk.referenced_columns)
    );
    if !fk.on_delete.is_empty() && fk.on_delete != "NO_ACTION" {
        write!(sql, " ON DELETE {}", fk.on_delete.replace('_', " ")).unwrap();
    }
    if !fk.on_update.is_empty() && fk.on_update != "NO_ACTION" {
        write!(sql, " ON UPDATE {}", fk.on_update.replace('_', " ")).unwrap();
    }
    sql.push(';');
    sql
}

fn uq_sql(schema: &str, table: &str, uq: &KeyConstraint) -> String {
    format!(
        "ALTER TABLE [{schema}].[{table}] ADD CONSTRAINT [{}] UNIQUE {} ({});",
        uq.name,
        clustered_kw(uq.clustered),
        cols_csv(&uq.columns)
    )
}

fn clustered_kw(c: bool) -> &'static str {
    if c { "CLUSTERED" } else { "NONCLUSTERED" }
}

fn cols_csv(cols: &[String]) -> String {
    cols.iter()
        .map(|c| format!("[{c}]"))
        .collect::<Vec<_>>()
        .join(", ")
}

// ---------- generic equality helper --------------------------------------

fn same_named<T: PartialEq>(haystack: Option<&[T]>, _name: &str, needle: &T) -> bool {
    match haystack {
        Some(items) => items.iter().any(|x| x == needle),
        None => false,
    }
}

// ---------- coverage warnings --------------------------------------------

fn collect_warnings(old: &SchemaGraph, new: &SchemaGraph, out: &mut Vec<String>) {
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for g in [old, new] {
        for s in &g.schemas {
            for t in &s.tables {
                for c in &t.columns {
                    if c.computed.is_some() {
                        seen.insert(format!(
                            "computed-column expression not canonicalized: {}.{}.{}",
                            s.name, t.name, c.name
                        ));
                    }
                }
            }
            if !s.types.is_empty() {
                seen.insert(format!(
                    "user-defined types not diffed in schema {}",
                    s.name
                ));
            }
        }
    }
    out.extend(seen);
}

#[cfg(test)]
mod tests {
    // Integration-style tests live in tests/diff.rs.
}
