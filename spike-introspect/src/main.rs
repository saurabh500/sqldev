// sqldev M0 spike: schema introspection over TDS via mssql-tiberius-bridge.
//
// Goal: connect to a SQL Server, walk system catalogs, and emit a JSON
// schema-graph (v0.1) on stdout. No CLI subcommands, no plugins, no diff —
// this exists only to validate the introspection path and the shape of the
// schema graph contract.

use anyhow::{Context, Result};
use clap::Parser;
use mssql_tiberius_bridge::{AuthMethod, Client, Config};
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Parser, Debug)]
#[command(version, about = "sqldev introspection spike")]
struct Args {
    /// Server host (without port).
    #[arg(long, default_value = "10.0.0.21")]
    host: String,

    /// TDS port.
    #[arg(long, default_value_t = 1434)]
    port: u16,

    /// SQL login username.
    #[arg(long, default_value = "SA")]
    user: String,

    /// SQL login password.
    #[arg(long)]
    password: String,

    /// Database to introspect.
    #[arg(long)]
    database: String,
}

// Schema graph contract v0.1: empty arrays are emitted as `[]`, never
// omitted. Optional scalar fields are still omitted when absent. This keeps
// downstream consumers (diff, codegen, jq) from having to special-case nulls.

#[derive(Serialize)]
struct SchemaGraph {
    version: &'static str,
    database: String,
    schemas: Vec<SchemaNode>,
    warnings: Vec<String>,
}

#[derive(Serialize)]
struct SchemaNode {
    name: String,
    tables: Vec<Table>,
    views: Vec<View>,
    procedures: Vec<Routine>,
    functions: Vec<Routine>,
    types: Vec<UserDefinedType>,
}

#[derive(Serialize)]
struct Table {
    name: String,
    columns: Vec<Column>,
    #[serde(skip_serializing_if = "Option::is_none")]
    primary_key: Option<KeyConstraint>,
    unique_constraints: Vec<KeyConstraint>,
    check_constraints: Vec<CheckConstraint>,
    foreign_keys: Vec<ForeignKey>,
    indexes: Vec<Index>,
    triggers: Vec<Trigger>,
}

#[derive(Serialize)]
struct Column {
    name: String,
    type_name: String,
    nullable: bool,
    identity: bool,
    /// True when type_name refers to a user-defined data type in this database.
    is_uddt: bool,
    /// For UDDT columns, the underlying system type formatted the same way as
    /// `type_name` would be for that base type. Absent for system-typed columns.
    #[serde(skip_serializing_if = "Option::is_none")]
    base_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    default: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    computed: Option<String>,
}

#[derive(Serialize)]
struct UserDefinedType {
    name: String,
    base_type: String,
    nullable: bool,
}

#[derive(Serialize)]
struct Trigger {
    name: String,
    /// `AFTER` | `INSTEAD_OF`.
    timing: &'static str,
    is_disabled: bool,
    definition: Option<String>,
}

#[derive(Serialize)]
struct KeyConstraint {
    name: String,
    columns: Vec<String>,
    clustered: bool,
}

#[derive(Serialize)]
struct CheckConstraint {
    name: String,
    expression: String,
}

#[derive(Serialize)]
struct ForeignKey {
    name: String,
    columns: Vec<String>,
    referenced_schema: String,
    referenced_table: String,
    referenced_columns: Vec<String>,
    on_delete: String,
    on_update: String,
}

#[derive(Serialize)]
struct Index {
    name: String,
    columns: Vec<String>,
    included_columns: Vec<String>,
    is_unique: bool,
    is_clustered: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    filter: Option<String>,
}

#[derive(Serialize)]
struct View {
    name: String,
    definition: Option<String>,
}

#[derive(Serialize)]
struct Routine {
    name: String,
    kind: &'static str,
    definition: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    let mut cfg = Config::new();
    cfg.host(&args.host)
        .port(args.port)
        .database(&args.database)
        .authentication(AuthMethod::sql_server(&args.user, &args.password))
        .trust_cert();

    let mut client = Client::connect(&cfg)
        .await
        .context("failed to connect to SQL Server")?;

    let started = std::time::Instant::now();

    // --- Schemas (filter to user schemas only). ---
    let schema_rows = client
        .simple_query(
            "SELECT s.schema_id, s.name
             FROM sys.schemas s
             WHERE s.name NOT IN ('sys','INFORMATION_SCHEMA','guest',
                                  'db_owner','db_accessadmin','db_securityadmin',
                                  'db_ddladmin','db_backupoperator','db_datareader',
                                  'db_datawriter','db_denydatareader','db_denydatawriter')
             ORDER BY s.name",
        )
        .await?
        .into_first_result();

    let mut schemas: BTreeMap<i32, SchemaNode> = BTreeMap::new();
    for r in &schema_rows {
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

    // --- User-defined types (alias / UDDT). Build a lookup keyed by
    // user_type_id so column rendering can distinguish a UDDT column from a
    // system-typed column and preserve the original case of the type name. ---
    struct UddtInfo {
        schema_id: i32,
        name: String,
        base_type: String,
        nullable: bool,
    }
    let uddt_rows = client
        .simple_query(
            "SELECT t.user_type_id, t.schema_id, t.name,
                    bt.name AS base_name,
                    t.max_length, t.precision, t.scale, t.is_nullable
             FROM sys.types t
             JOIN sys.types bt ON bt.user_type_id = t.system_type_id
             WHERE t.is_user_defined = 1 AND t.is_table_type = 0",
        )
        .await?
        .into_first_result();
    let mut uddts: BTreeMap<i32, UddtInfo> = BTreeMap::new();
    for r in &uddt_rows {
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
                base_type: format_type(base, max_length, precision, scale),
                nullable,
            },
        );
    }

    // --- Tables. ---
    struct TBuilder {
        schema_id: i32,
        name: String,
        columns: Vec<Column>,
        primary_key: Option<KeyConstraint>,
        unique_constraints: Vec<KeyConstraint>,
        check_constraints: Vec<CheckConstraint>,
        foreign_keys: Vec<ForeignKey>,
        indexes: Vec<Index>,
        triggers: Vec<Trigger>,
    }
    let mut tables: BTreeMap<i32, TBuilder> = BTreeMap::new();

    let table_rows = client
        .simple_query(
            "SELECT t.object_id, t.schema_id, t.name
             FROM sys.tables t
             WHERE t.is_ms_shipped = 0
             ORDER BY t.schema_id, t.name",
        )
        .await?
        .into_first_result();
    for r in &table_rows {
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
                name: name.to_string(),
                columns: vec![],
                primary_key: None,
                unique_constraints: vec![],
                check_constraints: vec![],
                foreign_keys: vec![],
                indexes: vec![],
                triggers: vec![],
            },
        );
    }

    // --- Columns. ---
    let column_rows = client
        .simple_query(
            "SELECT c.object_id, c.column_id, c.name,
                    c.user_type_id,
                    TYPE_NAME(c.user_type_id) AS type_base,
                    c.max_length, c.precision, c.scale,
                    c.is_nullable, c.is_identity,
                    OBJECT_DEFINITION(c.default_object_id) AS default_def,
                    cc.definition AS computed_def
             FROM sys.columns c
             LEFT JOIN sys.computed_columns cc
                    ON cc.object_id = c.object_id AND cc.column_id = c.column_id
             WHERE c.object_id IN (SELECT object_id FROM sys.tables WHERE is_ms_shipped = 0)
             ORDER BY c.object_id, c.column_id",
        )
        .await?
        .into_first_result();
    for r in &column_rows {
        let oid: i32 = r.get::<i32, _>("object_id").unwrap();
        let Some(t) = tables.get_mut(&oid) else { continue };
        let name: &str = r.get::<&str, _>("name").unwrap();
        let user_type_id: i32 = r.get::<i32, _>("user_type_id").unwrap_or(0);
        let base: &str = r.get::<&str, _>("type_base").unwrap_or("unknown");
        let max_length: i16 = r.get::<i16, _>("max_length").unwrap_or(0);
        let precision: u8 = r.get::<u8, _>("precision").unwrap_or(0);
        let scale: u8 = r.get::<u8, _>("scale").unwrap_or(0);
        let nullable: bool = r.get::<bool, _>("is_nullable").unwrap_or(false);
        let identity: bool = r.get::<bool, _>("is_identity").unwrap_or(false);
        let default = r.get::<&str, _>("default_def").map(strip_default_wrap);
        let computed = r.get::<&str, _>("computed_def").map(|s| s.to_string());
        let (type_name, is_uddt, base_type) = match uddts.get(&user_type_id) {
            Some(u) => (u.name.clone(), true, Some(u.base_type.clone())),
            None => (format_type(base, max_length, precision, scale), false, None),
        };
        t.columns.push(Column {
            name: name.to_string(),
            type_name,
            nullable,
            identity,
            is_uddt,
            base_type,
            default,
            computed,
        });
    }

    // --- Index columns (key + included), keyed by (object_id, index_id). ---
    let kcol_rows = client
        .simple_query(
            "SELECT ic.object_id AS parent_object_id, ic.index_id, ic.key_ordinal, c.name AS col_name
             FROM sys.index_columns ic
             JOIN sys.columns c
                  ON c.object_id = ic.object_id AND c.column_id = ic.column_id
             WHERE ic.is_included_column = 0
             ORDER BY ic.object_id, ic.index_id, ic.key_ordinal",
        )
        .await?
        .into_first_result();
    let mut idx_keycols: BTreeMap<(i32, i32), Vec<String>> = BTreeMap::new();
    for r in &kcol_rows {
        let oid: i32 = r.get::<i32, _>("parent_object_id").unwrap();
        let iid: i32 = r.get::<i32, _>("index_id").unwrap();
        let cname: &str = r.get::<&str, _>("col_name").unwrap();
        idx_keycols
            .entry((oid, iid))
            .or_default()
            .push(cname.to_string());
    }
    let inc_rows = client
        .simple_query(
            "SELECT ic.object_id AS parent_object_id, ic.index_id, c.name AS col_name
             FROM sys.index_columns ic
             JOIN sys.columns c
                  ON c.object_id = ic.object_id AND c.column_id = ic.column_id
             WHERE ic.is_included_column = 1
             ORDER BY ic.object_id, ic.index_id, ic.index_column_id",
        )
        .await?
        .into_first_result();
    let mut idx_inccols: BTreeMap<(i32, i32), Vec<String>> = BTreeMap::new();
    for r in &inc_rows {
        let oid: i32 = r.get::<i32, _>("parent_object_id").unwrap();
        let iid: i32 = r.get::<i32, _>("index_id").unwrap();
        let cname: &str = r.get::<&str, _>("col_name").unwrap();
        idx_inccols
            .entry((oid, iid))
            .or_default()
            .push(cname.to_string());
    }

    // --- Key constraints (PK + UQ). ---
    let kc_rows = client
        .simple_query(
            "SELECT kc.parent_object_id,
                    kc.name,
                    kc.type AS key_type,
                    i.index_id,
                    i.type_desc AS index_type_desc
             FROM sys.key_constraints kc
             JOIN sys.indexes i ON i.object_id = kc.parent_object_id AND i.name = kc.name
             ORDER BY kc.parent_object_id, kc.name",
        )
        .await?
        .into_first_result();
    for r in &kc_rows {
        let parent: i32 = r.get::<i32, _>("parent_object_id").unwrap();
        let Some(t) = tables.get_mut(&parent) else { continue };
        let name: &str = r.get::<&str, _>("name").unwrap();
        let ktype: &str = r.get::<&str, _>("key_type").unwrap();
        let iid: i32 = r.get::<i32, _>("index_id").unwrap();
        let type_desc: &str = r.get::<&str, _>("index_type_desc").unwrap_or("");
        let cols = idx_keycols
            .get(&(parent, iid))
            .cloned()
            .unwrap_or_default();
        let kc = KeyConstraint {
            name: name.trim().to_string(),
            columns: cols,
            clustered: type_desc == "CLUSTERED",
        };
        if ktype.trim() == "PK" {
            t.primary_key = Some(kc);
        } else {
            t.unique_constraints.push(kc);
        }
    }

    // --- All non-constraint indexes. ---
    let idx_rows = client
        .simple_query(
            "SELECT i.object_id AS parent_object_id,
                    i.index_id,
                    i.name,
                    i.is_unique,
                    i.type_desc,
                    i.has_filter,
                    i.filter_definition
             FROM sys.indexes i
             JOIN sys.tables t ON t.object_id = i.object_id
             WHERE i.is_primary_key = 0
               AND i.is_unique_constraint = 0
               AND i.type_desc <> 'HEAP'
               AND i.name IS NOT NULL
               AND t.is_ms_shipped = 0
             ORDER BY i.object_id, i.index_id",
        )
        .await?
        .into_first_result();
    for r in &idx_rows {
        let parent: i32 = r.get::<i32, _>("parent_object_id").unwrap();
        let Some(t) = tables.get_mut(&parent) else { continue };
        let iid: i32 = r.get::<i32, _>("index_id").unwrap();
        let name: &str = r.get::<&str, _>("name").unwrap();
        let is_unique: bool = r.get::<bool, _>("is_unique").unwrap_or(false);
        let type_desc: &str = r.get::<&str, _>("type_desc").unwrap_or("");
        let has_filter: bool = r.get::<bool, _>("has_filter").unwrap_or(false);
        let filter = if has_filter {
            r.get::<&str, _>("filter_definition")
                .map(|s| s.to_string())
        } else {
            None
        };
        let cols = idx_keycols
            .get(&(parent, iid))
            .cloned()
            .unwrap_or_default();
        let inc = idx_inccols
            .get(&(parent, iid))
            .cloned()
            .unwrap_or_default();
        t.indexes.push(Index {
            name: name.to_string(),
            columns: cols,
            included_columns: inc,
            is_unique,
            is_clustered: type_desc == "CLUSTERED",
            filter,
        });
    }

    // --- Check constraints. ---
    let cc_rows = client
        .simple_query(
            "SELECT cc.parent_object_id, cc.name, cc.definition
             FROM sys.check_constraints cc
             ORDER BY cc.parent_object_id, cc.name",
        )
        .await?
        .into_first_result();
    for r in &cc_rows {
        let parent: i32 = r.get::<i32, _>("parent_object_id").unwrap();
        let Some(t) = tables.get_mut(&parent) else { continue };
        let name: &str = r.get::<&str, _>("name").unwrap();
        let def: &str = r.get::<&str, _>("definition").unwrap_or("");
        t.check_constraints.push(CheckConstraint {
            name: name.to_string(),
            expression: def.to_string(),
        });
    }

    // --- Foreign keys. ---
    let fk_rows = client
        .simple_query(
            "SELECT fk.object_id AS fk_object_id,
                    fk.parent_object_id,
                    fk.name,
                    s2.name AS ref_schema,
                    t2.name AS ref_table,
                    fk.delete_referential_action_desc AS on_delete,
                    fk.update_referential_action_desc AS on_update
             FROM sys.foreign_keys fk
             JOIN sys.tables t2 ON t2.object_id = fk.referenced_object_id
             JOIN sys.schemas s2 ON s2.schema_id = t2.schema_id
             ORDER BY fk.parent_object_id, fk.name",
        )
        .await?
        .into_first_result();
    let fkc_rows = client
        .simple_query(
            "SELECT fkc.constraint_object_id,
                    fkc.constraint_column_id,
                    pc.name AS parent_col,
                    rc.name AS referenced_col
             FROM sys.foreign_key_columns fkc
             JOIN sys.columns pc
                  ON pc.object_id = fkc.parent_object_id AND pc.column_id = fkc.parent_column_id
             JOIN sys.columns rc
                  ON rc.object_id = fkc.referenced_object_id AND rc.column_id = fkc.referenced_column_id
             ORDER BY fkc.constraint_object_id, fkc.constraint_column_id",
        )
        .await?
        .into_first_result();
    let mut fk_cols: BTreeMap<i32, (Vec<String>, Vec<String>)> = BTreeMap::new();
    for r in &fkc_rows {
        let cid: i32 = r.get::<i32, _>("constraint_object_id").unwrap();
        let pc: &str = r.get::<&str, _>("parent_col").unwrap();
        let rc: &str = r.get::<&str, _>("referenced_col").unwrap();
        let entry = fk_cols.entry(cid).or_default();
        entry.0.push(pc.to_string());
        entry.1.push(rc.to_string());
    }
    for r in &fk_rows {
        let parent: i32 = r.get::<i32, _>("parent_object_id").unwrap();
        let Some(t) = tables.get_mut(&parent) else { continue };
        let fk_oid: i32 = r.get::<i32, _>("fk_object_id").unwrap();
        let name: &str = r.get::<&str, _>("name").unwrap();
        let ref_schema: &str = r.get::<&str, _>("ref_schema").unwrap();
        let ref_table: &str = r.get::<&str, _>("ref_table").unwrap();
        let on_delete: &str = r.get::<&str, _>("on_delete").unwrap_or("");
        let on_update: &str = r.get::<&str, _>("on_update").unwrap_or("");
        let (cols, refcols) = fk_cols.remove(&fk_oid).unwrap_or_default();
        t.foreign_keys.push(ForeignKey {
            name: name.to_string(),
            columns: cols,
            referenced_schema: ref_schema.to_string(),
            referenced_table: ref_table.to_string(),
            referenced_columns: refcols,
            on_delete: on_delete.to_string(),
            on_update: on_update.to_string(),
        });
    }

    // --- Triggers (DML triggers attached to user tables). ---
    let trig_rows = client
        .simple_query(
            "SELECT tr.parent_id AS parent_object_id,
                    tr.name,
                    tr.is_disabled,
                    tr.is_instead_of_trigger,
                    OBJECT_DEFINITION(tr.object_id) AS def
             FROM sys.triggers tr
             JOIN sys.tables t ON t.object_id = tr.parent_id
             WHERE tr.is_ms_shipped = 0 AND t.is_ms_shipped = 0
             ORDER BY tr.parent_id, tr.name",
        )
        .await?
        .into_first_result();
    for r in &trig_rows {
        let parent: i32 = r.get::<i32, _>("parent_object_id").unwrap();
        let Some(t) = tables.get_mut(&parent) else { continue };
        let name: &str = r.get::<&str, _>("name").unwrap();
        let is_disabled: bool = r.get::<bool, _>("is_disabled").unwrap_or(false);
        let is_instead_of: bool = r.get::<bool, _>("is_instead_of_trigger").unwrap_or(false);
        let def = r.get::<&str, _>("def").map(|s| s.to_string());
        t.triggers.push(Trigger {
            name: name.to_string(),
            timing: if is_instead_of { "INSTEAD_OF" } else { "AFTER" },
            is_disabled,
            definition: def,
        });
    }

    // --- Views, procedures, functions. ---
    let view_rows = client
        .simple_query(
            "SELECT v.schema_id, v.name, OBJECT_DEFINITION(v.object_id) AS def
             FROM sys.views v
             WHERE v.is_ms_shipped = 0
             ORDER BY v.schema_id, v.name",
        )
        .await?
        .into_first_result();
    let proc_rows = client
        .simple_query(
            "SELECT p.schema_id, p.name, OBJECT_DEFINITION(p.object_id) AS def
             FROM sys.procedures p
             WHERE p.is_ms_shipped = 0
             ORDER BY p.schema_id, p.name",
        )
        .await?
        .into_first_result();
    let fn_rows = client
        .simple_query(
            "SELECT o.schema_id, o.name, o.type AS routine_type, OBJECT_DEFINITION(o.object_id) AS def
             FROM sys.objects o
             WHERE o.type IN ('FN','IF','TF','FS','FT')
               AND o.is_ms_shipped = 0
             ORDER BY o.schema_id, o.name",
        )
        .await?
        .into_first_result();

    // Move table-builders into their schema nodes.
    for (_oid, b) in tables {
        if let Some(s) = schemas.get_mut(&b.schema_id) {
            s.tables.push(Table {
                name: b.name,
                columns: b.columns,
                primary_key: b.primary_key,
                unique_constraints: b.unique_constraints,
                check_constraints: b.check_constraints,
                foreign_keys: b.foreign_keys,
                indexes: b.indexes,
                triggers: b.triggers,
            });
        }
    }
    // UDDTs into their schema nodes.
    for (_utid, u) in uddts {
        if let Some(s) = schemas.get_mut(&u.schema_id) {
            s.types.push(UserDefinedType {
                name: u.name,
                base_type: u.base_type,
                nullable: u.nullable,
            });
        }
    }
    for r in &view_rows {
        let sid: i32 = r.get::<i32, _>("schema_id").unwrap();
        let name: &str = r.get::<&str, _>("name").unwrap();
        let def = r.get::<&str, _>("def").map(|s| s.to_string());
        if let Some(s) = schemas.get_mut(&sid) {
            s.views.push(View {
                name: name.to_string(),
                definition: def,
            });
        }
    }
    for r in &proc_rows {
        let sid: i32 = r.get::<i32, _>("schema_id").unwrap();
        let name: &str = r.get::<&str, _>("name").unwrap();
        let def = r.get::<&str, _>("def").map(|s| s.to_string());
        if let Some(s) = schemas.get_mut(&sid) {
            s.procedures.push(Routine {
                name: name.to_string(),
                kind: "procedure",
                definition: def,
            });
        }
    }
    for r in &fn_rows {
        let sid: i32 = r.get::<i32, _>("schema_id").unwrap();
        let name: &str = r.get::<&str, _>("name").unwrap();
        let kind = match r.get::<&str, _>("routine_type").unwrap_or("FN").trim() {
            "IF" => "inline-tvf",
            "TF" => "table-valued-function",
            "FS" => "clr-scalar-function",
            "FT" => "clr-table-function",
            _ => "scalar-function",
        };
        let def = r.get::<&str, _>("def").map(|s| s.to_string());
        if let Some(s) = schemas.get_mut(&sid) {
            s.functions.push(Routine {
                name: name.to_string(),
                kind,
                definition: def,
            });
        }
    }

    let mut schemas_vec: Vec<SchemaNode> = schemas.into_values().collect();
    for s in &mut schemas_vec {
        s.tables.sort_by(|a, b| a.name.cmp(&b.name));
        s.types.sort_by(|a, b| a.name.cmp(&b.name));
    }

    let graph = SchemaGraph {
        version: "0.1",
        database: args.database.clone(),
        schemas: schemas_vec,
        warnings: vec![],
    };

    println!("{}", serde_json::to_string_pretty(&graph)?);
    eprintln!(
        "[spike] introspected {} in {:?}",
        args.database,
        started.elapsed()
    );
    Ok(())
}

/// Format a SQL Server type name with length/precision/scale, matching the
/// shape developers actually write in CREATE TABLE statements.
fn format_type(base: &str, max_length: i16, precision: u8, scale: u8) -> String {
    let b = base.to_ascii_lowercase();
    match b.as_str() {
        "varchar" | "char" | "varbinary" | "binary" => {
            if max_length == -1 {
                format!("{}(max)", b)
            } else {
                format!("{}({})", b, max_length)
            }
        }
        "nvarchar" | "nchar" => {
            if max_length == -1 {
                format!("{}(max)", b)
            } else {
                // nvarchar max_length is in bytes; halve for char count.
                format!("{}({})", b, max_length / 2)
            }
        }
        "decimal" | "numeric" => format!("{}({},{})", b, precision, scale),
        "datetime2" | "datetimeoffset" | "time" => {
            if scale == 7 {
                b
            } else {
                format!("{}({})", b, scale)
            }
        }
        "float" => {
            if precision == 53 {
                "float".into()
            } else {
                format!("float({})", precision)
            }
        }
        _ => b,
    }
}

/// SQL Server wraps default expressions in extra parens (e.g. `((0))`).
/// Strip them for readability.
fn strip_default_wrap(s: &str) -> String {
    let mut t = s.trim().to_string();
    while t.len() >= 2 && t.starts_with('(') && t.ends_with(')') {
        // Only strip a balanced outer pair.
        let mut depth = 0i32;
        let mut balanced_outer = true;
        for (i, c) in t.chars().enumerate() {
            match c {
                '(' => depth += 1,
                ')' => depth -= 1,
                _ => {}
            }
            if depth == 0 && i + 1 < t.len() {
                balanced_outer = false;
                break;
            }
        }
        if balanced_outer {
            t = t[1..t.len() - 1].trim().to_string();
        } else {
            break;
        }
    }
    t
}
