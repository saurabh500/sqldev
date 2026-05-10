//! Schema graph contract — version 0.1.
//!
//! Stability rules:
//! - Empty arrays MUST serialize as `[]` (no `skip_serializing_if` on `Vec`).
//!   Downstream tooling (`diff`, codegen, jq) is allowed to assume the field
//!   exists.
//! - Optional scalars MAY be skipped via `Option::is_none`.
//! - Field renames are breaking. Bumping `version` is the contract.
//!
//! This shape was validated against `AdventureWorks2022` in the M0 spike:
//! 71 tables, 486 columns, 90 FKs, 89 checks, 101 indexes, 6 UDDTs, 10
//! triggers — round-tripped through `spike-diff` with 8/8 passing migrations.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchemaGraph {
    pub version: String,
    pub database: String,
    pub schemas: Vec<SchemaNode>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchemaNode {
    pub name: String,
    pub tables: Vec<Table>,
    pub views: Vec<View>,
    pub procedures: Vec<Routine>,
    pub functions: Vec<Routine>,
    pub types: Vec<UserDefinedType>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Table {
    pub name: String,
    pub columns: Vec<Column>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub primary_key: Option<KeyConstraint>,
    pub unique_constraints: Vec<KeyConstraint>,
    pub check_constraints: Vec<CheckConstraint>,
    pub foreign_keys: Vec<ForeignKey>,
    pub indexes: Vec<Index>,
    pub triggers: Vec<Trigger>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Column {
    pub name: String,
    pub type_name: String,
    pub nullable: bool,
    pub identity: bool,
    /// True when `type_name` refers to a user-defined data type in this
    /// database. The diff layer needs this to know whether `type_name` is a
    /// system type or `[schema].[name]`.
    pub is_uddt: bool,
    /// For UDDT columns, the underlying system type formatted the same way
    /// as `type_name` would be for that base type. Absent for system-typed
    /// columns.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub base_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub default: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub computed: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UserDefinedType {
    pub name: String,
    pub base_type: String,
    pub nullable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Trigger {
    pub name: String,
    /// Either `"AFTER"` or `"INSTEAD_OF"`. Stored as a string (not enum) so
    /// JSON consumers don't have to know the closed set; the diff layer
    /// matches on the literal.
    pub timing: String,
    pub is_disabled: bool,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub definition: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct KeyConstraint {
    pub name: String,
    pub columns: Vec<String>,
    pub clustered: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CheckConstraint {
    pub name: String,
    pub expression: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ForeignKey {
    pub name: String,
    pub columns: Vec<String>,
    pub referenced_schema: String,
    pub referenced_table: String,
    pub referenced_columns: Vec<String>,
    pub on_delete: String,
    pub on_update: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Index {
    pub name: String,
    pub columns: Vec<String>,
    pub included_columns: Vec<String>,
    pub is_unique: bool,
    pub is_clustered: bool,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub filter: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct View {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub definition: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Routine {
    pub name: String,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub definition: Option<String>,
}

/// Current schema graph version. Bump on any breaking field change.
pub const SCHEMA_GRAPH_VERSION: &str = "0.1";
