//! Schema-graph builder.
//!
//! Walks the SQL Server `sys.*` catalog and assembles a
//! [`sqldev_core::SchemaGraph`]. The query set, edge cases (UDDTs, computed
//! columns, filtered indexes, INCLUDE columns, triggers), and contract
//! decisions were settled in the M0 spike — see the proposal repo's
//! `spike-introspect/` for the validation history.

#![forbid(unsafe_code)]

mod queries;
mod typefmt;

use mssql_tiberius_bridge::Client;
use sqldev_core::{
    CheckConstraint, Column, Error, ForeignKey, Index, KeyConstraint, Result, Routine, SchemaGraph,
    SchemaNode, Table, Trigger, UserDefinedType, View, schema::SCHEMA_GRAPH_VERSION,
};
use std::collections::BTreeMap;

// Internal aggregation helpers — kept private to this module.
struct UddtInfo {
    schema_id: i32,
    name: String,
    base_type: String,
    nullable: bool,
}

struct TBuilder {
    schema_id: i32,
    table: Table,
}

/// Build the schema graph for `database` using the supplied client.
///
/// The `client` is borrowed mutably for the duration of the calls and is
/// left open afterwards — the caller decides when to drop it.
#[allow(clippy::too_many_lines)] // catalog walk is sequential; splitting hurts readability
pub async fn build_schema_graph(client: &mut Client, database: &str) -> Result<SchemaGraph> {
    let mut schemas: BTreeMap<i32, SchemaNode> = BTreeMap::new();

    // --- Schemas (filter to user schemas only). ---
    for r in &fetch(client, queries::SCHEMAS).await? {
        let id: i32 = r.get::<i32, _>("schema_id").unwrap();
        let name: &str = r.get::<&str, _>("name").unwrap();
        schemas.insert(
            id,
            SchemaNode {
                name: name.to_string(),
                tables: vec![],
                views: vec![],
                procedures: vec![],
                functions: vec![],
                types: vec![],
            },
        );
    }

    // --- UDDTs. Build a lookup by user_type_id. ---
    let mut uddts: BTreeMap<i32, UddtInfo> = BTreeMap::new();
    for r in &fetch(client, queries::UDDTS).await? {
        let utid: i32 = r.get::<i32, _>("user_type_id").unwrap();
        let sid: i32 = r.get::<i32, _>("schema_id").unwrap();
        let name: &str = r.get::<&str, _>("name").unwrap();
        let base: &str = r.get::<&str, _>("base_name").unwrap_or("unknown");
        let max_length: i16 = r.get::<i16, _>("max_length").unwrap_or(0);
        let precision: u8 = r.get::<u8, _>("precision").unwrap_or(0);
        let scale: u8 = r.get::<u8, _>("scale").unwrap_or(0);
        let nullable: bool = r.get::<bool, _>("is_nullable").unwrap_or(false);
        uddts.insert(
            utid,
            UddtInfo {
                schema_id: sid,
                name: name.to_string(),
                base_type: typefmt::format_type(base, max_length, precision, scale),
                nullable,
            },
        );
    }

    // --- Tables. ---
    let mut tables: BTreeMap<i32, TBuilder> = BTreeMap::new();
    for r in &fetch(client, queries::TABLES).await? {
        let oid: i32 = r.get::<i32, _>("object_id").unwrap();
        let sid: i32 = r.get::<i32, _>("schema_id").unwrap();
        let name: &str = r.get::<&str, _>("name").unwrap();
        if !schemas.contains_key(&sid) {
            continue;
        }
        tables.insert(
            oid,
            TBuilder {
                schema_id: sid,
                table: Table {
                    name: name.to_string(),
                    columns: vec![],
                    primary_key: None,
                    unique_constraints: vec![],
                    check_constraints: vec![],
                    foreign_keys: vec![],
                    indexes: vec![],
                    triggers: vec![],
                },
            },
        );
    }

    // --- Columns. ---
    for r in &fetch(client, queries::COLUMNS).await? {
        let oid: i32 = r.get::<i32, _>("object_id").unwrap();
        let Some(t) = tables.get_mut(&oid) else {
            continue;
        };
        let name: &str = r.get::<&str, _>("name").unwrap();
        let user_type_id: i32 = r.get::<i32, _>("user_type_id").unwrap_or(0);
        let base: &str = r.get::<&str, _>("type_base").unwrap_or("unknown");
        let max_length: i16 = r.get::<i16, _>("max_length").unwrap_or(0);
        let precision: u8 = r.get::<u8, _>("precision").unwrap_or(0);
        let scale: u8 = r.get::<u8, _>("scale").unwrap_or(0);
        let nullable: bool = r.get::<bool, _>("is_nullable").unwrap_or(false);
        let identity: bool = r.get::<bool, _>("is_identity").unwrap_or(false);
        let default = r
            .get::<&str, _>("default_def")
            .map(typefmt::strip_default_wrap);
        let computed = r.get::<&str, _>("computed_def").map(str::to_string);
        let (type_name, is_uddt, udt_schema, base_type) = match uddts.get(&user_type_id) {
            Some(u) => (
                u.name.clone(),
                true,
                schemas.get(&u.schema_id).map(|s| s.name.clone()),
                Some(u.base_type.clone()),
            ),
            None => (
                typefmt::format_type(base, max_length, precision, scale),
                false,
                None,
                None,
            ),
        };
        t.table.columns.push(Column {
            name: name.to_string(),
            type_name,
            nullable,
            identity,
            is_uddt,
            udt_schema,
            base_type,
            default,
            computed,
        });
    }

    // --- Index columns (key + included). ---
    let mut idx_keycols: BTreeMap<(i32, i32), Vec<String>> = BTreeMap::new();
    for r in &fetch(client, queries::INDEX_KEY_COLS).await? {
        let oid: i32 = r.get::<i32, _>("parent_object_id").unwrap();
        let iid: i32 = r.get::<i32, _>("index_id").unwrap();
        let cname: &str = r.get::<&str, _>("col_name").unwrap();
        idx_keycols
            .entry((oid, iid))
            .or_default()
            .push(cname.to_string());
    }
    let mut idx_inccols: BTreeMap<(i32, i32), Vec<String>> = BTreeMap::new();
    for r in &fetch(client, queries::INDEX_INC_COLS).await? {
        let oid: i32 = r.get::<i32, _>("parent_object_id").unwrap();
        let iid: i32 = r.get::<i32, _>("index_id").unwrap();
        let cname: &str = r.get::<&str, _>("col_name").unwrap();
        idx_inccols
            .entry((oid, iid))
            .or_default()
            .push(cname.to_string());
    }

    // --- PK + UQ key constraints. ---
    for r in &fetch(client, queries::KEY_CONSTRAINTS).await? {
        let parent: i32 = r.get::<i32, _>("parent_object_id").unwrap();
        let Some(t) = tables.get_mut(&parent) else {
            continue;
        };
        let name: &str = r.get::<&str, _>("name").unwrap();
        let ktype: &str = r.get::<&str, _>("key_type").unwrap();
        let iid: i32 = r.get::<i32, _>("index_id").unwrap();
        let type_desc: &str = r.get::<&str, _>("index_type_desc").unwrap_or("");
        let cols = idx_keycols.get(&(parent, iid)).cloned().unwrap_or_default();
        let kc = KeyConstraint {
            name: name.trim().to_string(),
            columns: cols,
            clustered: type_desc == "CLUSTERED",
        };
        if ktype.trim() == "PK" {
            t.table.primary_key = Some(kc);
        } else {
            t.table.unique_constraints.push(kc);
        }
    }

    // --- All non-constraint indexes. ---
    for r in &fetch(client, queries::INDEXES).await? {
        let parent: i32 = r.get::<i32, _>("parent_object_id").unwrap();
        let Some(t) = tables.get_mut(&parent) else {
            continue;
        };
        let iid: i32 = r.get::<i32, _>("index_id").unwrap();
        let name: &str = r.get::<&str, _>("name").unwrap();
        let is_unique: bool = r.get::<bool, _>("is_unique").unwrap_or(false);
        let type_desc: &str = r.get::<&str, _>("type_desc").unwrap_or("");
        let has_filter: bool = r.get::<bool, _>("has_filter").unwrap_or(false);
        let filter = if has_filter {
            r.get::<&str, _>("filter_definition").map(str::to_string)
        } else {
            None
        };
        let cols = idx_keycols.get(&(parent, iid)).cloned().unwrap_or_default();
        let inc = idx_inccols.get(&(parent, iid)).cloned().unwrap_or_default();
        t.table.indexes.push(Index {
            name: name.to_string(),
            columns: cols,
            included_columns: inc,
            is_unique,
            is_clustered: type_desc == "CLUSTERED",
            filter,
        });
    }

    // --- Check constraints. ---
    for r in &fetch(client, queries::CHECK_CONSTRAINTS).await? {
        let parent: i32 = r.get::<i32, _>("parent_object_id").unwrap();
        let Some(t) = tables.get_mut(&parent) else {
            continue;
        };
        let name: &str = r.get::<&str, _>("name").unwrap();
        let def: &str = r.get::<&str, _>("definition").unwrap_or("");
        t.table.check_constraints.push(CheckConstraint {
            name: name.to_string(),
            expression: def.to_string(),
        });
    }

    // --- Foreign keys (two-step: header + columns). ---
    let mut fk_cols: BTreeMap<i32, (Vec<String>, Vec<String>)> = BTreeMap::new();
    for r in &fetch(client, queries::FK_COLUMNS).await? {
        let cid: i32 = r.get::<i32, _>("constraint_object_id").unwrap();
        let pc: &str = r.get::<&str, _>("parent_col").unwrap();
        let rc: &str = r.get::<&str, _>("referenced_col").unwrap();
        let entry = fk_cols.entry(cid).or_default();
        entry.0.push(pc.to_string());
        entry.1.push(rc.to_string());
    }
    for r in &fetch(client, queries::FOREIGN_KEYS).await? {
        let parent: i32 = r.get::<i32, _>("parent_object_id").unwrap();
        let Some(t) = tables.get_mut(&parent) else {
            continue;
        };
        let fk_oid: i32 = r.get::<i32, _>("fk_object_id").unwrap();
        let name: &str = r.get::<&str, _>("name").unwrap();
        let ref_schema: &str = r.get::<&str, _>("ref_schema").unwrap();
        let ref_table: &str = r.get::<&str, _>("ref_table").unwrap();
        let on_delete: &str = r.get::<&str, _>("on_delete").unwrap_or("");
        let on_update: &str = r.get::<&str, _>("on_update").unwrap_or("");
        let (cols, refcols) = fk_cols.remove(&fk_oid).unwrap_or_default();
        t.table.foreign_keys.push(ForeignKey {
            name: name.to_string(),
            columns: cols,
            referenced_schema: ref_schema.to_string(),
            referenced_table: ref_table.to_string(),
            referenced_columns: refcols,
            on_delete: on_delete.to_string(),
            on_update: on_update.to_string(),
        });
    }

    // --- Triggers. ---
    for r in &fetch(client, queries::TRIGGERS).await? {
        let parent: i32 = r.get::<i32, _>("parent_object_id").unwrap();
        let Some(t) = tables.get_mut(&parent) else {
            continue;
        };
        let name: &str = r.get::<&str, _>("name").unwrap();
        let is_disabled: bool = r.get::<bool, _>("is_disabled").unwrap_or(false);
        let is_instead_of: bool = r.get::<bool, _>("is_instead_of_trigger").unwrap_or(false);
        let def = r.get::<&str, _>("def").map(str::to_string);
        t.table.triggers.push(Trigger {
            name: name.to_string(),
            timing: if is_instead_of { "INSTEAD_OF" } else { "AFTER" }.to_string(),
            is_disabled,
            definition: def,
        });
    }

    // Move table-builders into their schema nodes.
    for (_oid, b) in tables {
        if let Some(s) = schemas.get_mut(&b.schema_id) {
            s.tables.push(b.table);
        }
    }
    for (_utid, u) in uddts {
        if let Some(s) = schemas.get_mut(&u.schema_id) {
            s.types.push(UserDefinedType {
                name: u.name,
                base_type: u.base_type,
                nullable: u.nullable,
            });
        }
    }

    // --- Views. ---
    for r in &fetch(client, queries::VIEWS).await? {
        let sid: i32 = r.get::<i32, _>("schema_id").unwrap();
        let name: &str = r.get::<&str, _>("name").unwrap();
        let def = r.get::<&str, _>("def").map(str::to_string);
        if let Some(s) = schemas.get_mut(&sid) {
            s.views.push(View {
                name: name.to_string(),
                definition: def,
            });
        }
    }
    // --- Procedures. ---
    for r in &fetch(client, queries::PROCEDURES).await? {
        let sid: i32 = r.get::<i32, _>("schema_id").unwrap();
        let name: &str = r.get::<&str, _>("name").unwrap();
        let def = r.get::<&str, _>("def").map(str::to_string);
        if let Some(s) = schemas.get_mut(&sid) {
            s.procedures.push(Routine {
                name: name.to_string(),
                kind: "procedure".to_string(),
                definition: def,
            });
        }
    }
    // --- Functions. ---
    for r in &fetch(client, queries::FUNCTIONS).await? {
        let sid: i32 = r.get::<i32, _>("schema_id").unwrap();
        let name: &str = r.get::<&str, _>("name").unwrap();
        let kind = match r.get::<&str, _>("routine_type").unwrap_or("FN").trim() {
            "IF" => "inline-tvf",
            "TF" => "table-valued-function",
            "FS" => "clr-scalar-function",
            "FT" => "clr-table-function",
            _ => "scalar-function",
        };
        let def = r.get::<&str, _>("def").map(str::to_string);
        if let Some(s) = schemas.get_mut(&sid) {
            s.functions.push(Routine {
                name: name.to_string(),
                kind: kind.to_string(),
                definition: def,
            });
        }
    }

    let mut schemas_vec: Vec<SchemaNode> = schemas.into_values().collect();
    for s in &mut schemas_vec {
        s.tables.sort_by(|a, b| a.name.cmp(&b.name));
        s.types.sort_by(|a, b| a.name.cmp(&b.name));
    }

    Ok(SchemaGraph {
        version: SCHEMA_GRAPH_VERSION.to_string(),
        database: database.to_string(),
        schemas: schemas_vec,
        warnings: vec![],
    })
}

/// Run a `simple_query` and surface failure as [`Error::Introspect`].
async fn fetch(client: &mut Client, sql: &str) -> Result<Vec<mssql_tiberius_bridge::Row>> {
    let rows = client
        .simple_query(sql)
        .await
        .map_err(|e| Error::Introspect(e.to_string()))?
        .into_first_result();
    Ok(rows)
}
