//! YAML-driven, type-aware fake-data generator for `sqldev seed`.
//!
//! Public API:
//! - [`SeedFile`] / [`ColumnSpec`]: the on-disk YAML shape.
//! - [`load_seed_file`] / [`load_seed_dir`]: parse YAML on disk.
//! - [`Generator`]: deterministic random source with a faker vocabulary.
//! - [`build_inserts`]: glue that turns a `SeedFile` + the target [`Table`]
//!   from a `SchemaGraph` into a sequence of `INSERT` batches.
//!
//! Determinism contract: given the same `--seed N`, the generator emits the
//! same SQL byte-for-byte. We use [`rand_chacha::ChaCha20Rng`] (not
//! `StdRng`) because `StdRng` is allowed to change its internal algorithm
//! across `rand` versions.

#![forbid(unsafe_code)]

mod faker;

pub use faker::FAKERS;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha20Rng;
use serde::{Deserialize, Serialize};
use sqldev_core::{Column, Table};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SeedError {
    #[error("io error reading {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("yaml parse error in {path}: {source}")]
    Yaml {
        path: PathBuf,
        #[source]
        source: serde_yaml::Error,
    },
    #[error("seed file references unknown table `{0}`")]
    UnknownTable(String),
    #[error("seed file `{file}` references unknown column `{column}` on `{table}`")]
    UnknownColumn {
        file: PathBuf,
        table: String,
        column: String,
    },
    #[error("unsupported faker tag `{0}`")]
    UnknownFaker(String),
}

/// One YAML seed file.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct SeedFile {
    /// Fully-qualified table name, `schema.table` (`schema` defaults to
    /// `dbo` when omitted).
    pub table: String,
    /// How many rows to insert.
    pub rows: u32,
    /// Per-column overrides; columns not listed get type-aware defaults.
    #[serde(default)]
    pub columns: BTreeMap<String, ColumnSpec>,
    /// Number of rows per `INSERT ... VALUES (...)` batch. Defaults to
    /// 100 (well under SQL Server's 1000-row VALUES limit).
    #[serde(default)]
    pub batch_size: Option<u32>,
}

/// Override knobs for a single column.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
pub struct ColumnSpec {
    /// A faker tag like `name`, `email`, `city`. See [`faker::FAKERS`] for
    /// the supported set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub faker: Option<String>,
    /// A literal value to splat into every row. Strings are quoted with
    /// `N'...'` and `'` is escaped. Numbers / booleans / `null` come
    /// through as-is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<serde_yaml::Value>,
    /// 0.0–1.0 probability of emitting `NULL` (only meaningful for
    /// nullable columns).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub null_ratio: Option<f64>,
}

/// Parse one YAML file into a [`SeedFile`].
pub fn load_seed_file(path: impl AsRef<Path>) -> Result<SeedFile, SeedError> {
    let path = path.as_ref();
    let text = std::fs::read_to_string(path).map_err(|e| SeedError::Io {
        path: path.into(),
        source: e,
    })?;
    serde_yaml::from_str(&text).map_err(|e| SeedError::Yaml {
        path: path.into(),
        source: e,
    })
}

/// Read every `*.yml` / `*.yaml` in `dir`, returned in lexicographic order
/// (matching `seeds/0001_*.yml` ordering conventions).
pub fn load_seed_dir(dir: impl AsRef<Path>) -> Result<Vec<(PathBuf, SeedFile)>, SeedError> {
    let dir = dir.as_ref();
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| SeedError::Io {
            path: dir.into(),
            source: e,
        })?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            let lower = p
                .extension()
                .and_then(|e| e.to_str())
                .map(str::to_ascii_lowercase);
            matches!(lower.as_deref(), Some("yml" | "yaml"))
        })
        .collect();
    entries.sort();
    let mut out = Vec::with_capacity(entries.len());
    for path in entries {
        let seed = load_seed_file(&path)?;
        out.push((path, seed));
    }
    Ok(out)
}

/// Split a fully-qualified `schema.table` (or bare `table`, defaulting to
/// `dbo`) into its parts. `[bracketed]` names are tolerated.
#[must_use]
pub fn split_qualified(name: &str) -> (String, String) {
    let trim = |s: &str| {
        s.trim()
            .trim_start_matches('[')
            .trim_end_matches(']')
            .to_string()
    };
    if let Some((s, t)) = name.split_once('.') {
        (trim(s), trim(t))
    } else {
        ("dbo".into(), trim(name))
    }
}

/// Deterministic random source with a faker vocabulary.
pub struct Generator {
    rng: ChaCha20Rng,
}

impl Generator {
    /// Reproducible across runs. Calls with the same seed produce identical
    /// outputs.
    #[must_use]
    pub fn from_seed(seed: u64) -> Self {
        Self {
            rng: ChaCha20Rng::seed_from_u64(seed),
        }
    }

    /// Non-deterministic (entropy-seeded) generator.
    #[must_use]
    pub fn from_entropy() -> Self {
        Self {
            rng: ChaCha20Rng::from_entropy(),
        }
    }

    /// Run the named faker, returning a SQL literal token (already
    /// quoted/escaped). `len_hint` is the maximum string length the column
    /// can hold; the faker may truncate.
    ///
    /// # Errors
    /// Returns [`SeedError::UnknownFaker`] for unknown tags.
    pub fn run_faker(&mut self, tag: &str, len_hint: Option<usize>) -> Result<String, SeedError> {
        faker::generate(tag, &mut self.rng, len_hint)
            .ok_or_else(|| SeedError::UnknownFaker(tag.to_string()))
    }

    /// Generate a value appropriate for `column`'s SQL type. Used when the
    /// seed file gives no explicit override.
    pub fn type_aware(&mut self, column: &Column) -> String {
        // Strip any trailing `(n)` / `(n,m)` / `MAX` for the lookup.
        let base = column
            .type_name
            .split('(')
            .next()
            .unwrap_or(&column.type_name)
            .trim()
            .to_ascii_lowercase();
        let len = parse_string_len(&column.type_name);
        match base.as_str() {
            "bit" => {
                if self.rng.r#gen::<bool>() {
                    "1".into()
                } else {
                    "0".into()
                }
            }
            "tinyint" => self.rng.gen_range(0u32..=255).to_string(),
            "smallint" => self.rng.gen_range(-32_768i32..=32_767).to_string(),
            "int" => self.rng.gen_range(0i32..1_000_000).to_string(),
            "bigint" => self.rng.gen_range(0i64..1_000_000_000).to_string(),
            "decimal" | "numeric" | "money" | "smallmoney" => {
                format!("{:.2}", self.rng.gen_range(0.0f64..10_000.0))
            }
            "float" | "real" => format!("{:.4}", self.rng.gen_range(0.0f64..10_000.0)),
            "uniqueidentifier" => format!("'{}'", faker::uuid_v4(&mut self.rng)),
            "date" => format!("'{}'", faker::random_date(&mut self.rng)),
            "time" => format!("'{}'", faker::random_time(&mut self.rng)),
            "datetime" | "datetime2" | "smalldatetime" | "datetimeoffset" => {
                format!(
                    "'{} {}'",
                    faker::random_date(&mut self.rng),
                    faker::random_time(&mut self.rng)
                )
            }
            // Default: short faker word, length-clipped.
            _ => faker::quoted_word(&mut self.rng, len),
        }
    }
}

fn parse_string_len(type_name: &str) -> Option<usize> {
    // Match `varchar(50)`, `nvarchar(MAX)`, `char(10)`. Returns None for
    // MAX or unparseable.
    let open = type_name.find('(')?;
    let close = type_name.find(')')?;
    let inner = type_name.get(open + 1..close)?.trim();
    if inner.eq_ignore_ascii_case("max") {
        return None;
    }
    inner.split(',').next()?.trim().parse().ok()
}

/// Render a YAML scalar as a SQL literal token. Strings → `N'...'`,
/// numbers/bools as-is, null/missing → `NULL`.
fn yaml_to_sql(value: &serde_yaml::Value) -> String {
    match value {
        serde_yaml::Value::Null => "NULL".into(),
        serde_yaml::Value::Bool(true) => "1".into(),
        serde_yaml::Value::Bool(false) => "0".into(),
        serde_yaml::Value::Number(n) => n.to_string(),
        serde_yaml::Value::String(s) => quote_nstring(s),
        // Sequences / mappings as JSON-y string fallback.
        other => quote_nstring(&serde_yaml::to_string(other).unwrap_or_default()),
    }
}

fn quote_nstring(s: &str) -> String {
    format!("N'{}'", s.replace('\'', "''"))
}

fn quote_ident(s: &str) -> String {
    format!("[{}]", s.replace(']', "]]"))
}

/// One generated batch: an `INSERT INTO ... VALUES (...), (...)...` ready
/// to ship to `simple_query`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InsertBatch {
    pub schema: String,
    pub table: String,
    pub sql: String,
    pub row_count: usize,
}

/// Build the per-row literals for `seed`, skipping identity / computed
/// columns (the DB assigns those). Each inner `Vec` has one entry per
/// non-skipped column in `table.columns` order.
pub fn build_rows(
    seed: &SeedFile,
    table: &Table,
    file_path: &Path,
    g: &mut Generator,
) -> Result<Vec<Vec<String>>, SeedError> {
    // Validate every override targets a real column.
    for name in seed.columns.keys() {
        if !table.columns.iter().any(|c| &c.name == name) {
            return Err(SeedError::UnknownColumn {
                file: file_path.into(),
                table: seed.table.clone(),
                column: name.clone(),
            });
        }
    }

    let active: Vec<&Column> = table
        .columns
        .iter()
        .filter(|c| !c.identity && c.computed.is_none())
        .collect();

    let mut rows = Vec::with_capacity(seed.rows as usize);
    for _ in 0..seed.rows {
        let mut row = Vec::with_capacity(active.len());
        for col in &active {
            let spec = seed.columns.get(&col.name);
            let null_roll = spec
                .and_then(|s| s.null_ratio)
                .filter(|r| col.nullable && *r > 0.0);
            let force_null = match null_roll {
                Some(r) => g.rng.r#gen::<f64>() < r,
                None => false,
            };
            if force_null {
                row.push("NULL".into());
                continue;
            }
            let lit = if let Some(spec) = spec {
                if let Some(v) = &spec.value {
                    yaml_to_sql(v)
                } else if let Some(tag) = &spec.faker {
                    g.run_faker(tag, parse_string_len(&col.type_name))?
                } else {
                    g.type_aware(col)
                }
            } else {
                g.type_aware(col)
            };
            row.push(lit);
        }
        rows.push(row);
    }
    Ok(rows)
}

/// Assemble [`InsertBatch`]es from the rows produced by [`build_rows`].
pub fn build_inserts(
    seed: &SeedFile,
    table: &Table,
    file_path: &Path,
    g: &mut Generator,
) -> Result<Vec<InsertBatch>, SeedError> {
    let rows = build_rows(seed, table, file_path, g)?;
    if rows.is_empty() {
        return Ok(vec![]);
    }
    let active: Vec<&Column> = table
        .columns
        .iter()
        .filter(|c| !c.identity && c.computed.is_none())
        .collect();
    let (schema_name, table_name) = split_qualified(&seed.table);
    let cols_sql = active
        .iter()
        .map(|c| quote_ident(&c.name))
        .collect::<Vec<_>>()
        .join(", ");
    let prefix = format!(
        "INSERT INTO {}.{} ({cols_sql}) VALUES",
        quote_ident(&schema_name),
        quote_ident(&table_name),
    );
    let batch_size = seed.batch_size.unwrap_or(100).max(1) as usize;

    let mut out = Vec::new();
    for chunk in rows.chunks(batch_size) {
        use std::fmt::Write as _;
        let mut sql = String::with_capacity(prefix.len() + chunk.len() * 64);
        sql.push_str(&prefix);
        sql.push('\n');
        for (i, row) in chunk.iter().enumerate() {
            if i > 0 {
                sql.push_str(",\n");
            }
            write!(sql, "  ({})", row.join(", ")).unwrap();
        }
        sql.push(';');
        out.push(InsertBatch {
            schema: schema_name.clone(),
            table: table_name.clone(),
            sql,
            row_count: chunk.len(),
        });
    }
    Ok(out)
}

/// Look up a `schema.table` in a `SchemaGraph`, tolerating `dbo.` shorthand.
#[must_use]
pub fn find_table<'g>(graph: &'g sqldev_core::SchemaGraph, qualified: &str) -> Option<&'g Table> {
    let (s, t) = split_qualified(qualified);
    graph
        .schemas
        .iter()
        .find(|sn| sn.name == s)
        .and_then(|sn| sn.tables.iter().find(|tab| tab.name == t))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn col(name: &str, type_name: &str, nullable: bool) -> Column {
        Column {
            name: name.into(),
            type_name: type_name.into(),
            nullable,
            identity: false,
            is_uddt: false,
            base_type: None,
            default: None,
            computed: None,
        }
    }

    fn table(name: &str, columns: Vec<Column>) -> Table {
        Table {
            name: name.into(),
            columns,
            primary_key: None,
            unique_constraints: vec![],
            check_constraints: vec![],
            foreign_keys: vec![],
            indexes: vec![],
            triggers: vec![],
        }
    }

    #[test]
    fn split_qualified_handles_brackets_and_default_schema() {
        assert_eq!(
            split_qualified("dbo.Customer"),
            ("dbo".into(), "Customer".into())
        );
        assert_eq!(
            split_qualified("Customer"),
            ("dbo".into(), "Customer".into())
        );
        assert_eq!(
            split_qualified("[sales].[Order]"),
            ("sales".into(), "Order".into())
        );
    }

    #[test]
    fn parse_string_len_parses_varchar_n() {
        assert_eq!(parse_string_len("nvarchar(50)"), Some(50));
        assert_eq!(parse_string_len("varchar(MAX)"), None);
        assert_eq!(parse_string_len("int"), None);
    }

    #[test]
    fn quote_helpers_escape_quotes_and_brackets() {
        assert_eq!(quote_nstring("O'Brien"), "N'O''Brien'");
        assert_eq!(quote_ident("we[i]rd"), "[we[i]]rd]");
    }

    #[test]
    fn deterministic_seed_round_trips() {
        let t = table(
            "Customer",
            vec![
                Column {
                    identity: true,
                    ..col("Id", "int", false)
                },
                col("Name", "nvarchar(50)", false),
                col("Age", "int", false),
            ],
        );
        let seed = SeedFile {
            table: "dbo.Customer".into(),
            rows: 3,
            columns: BTreeMap::new(),
            batch_size: None,
        };

        let mut g1 = Generator::from_seed(42);
        let mut g2 = Generator::from_seed(42);
        let a = build_inserts(&seed, &t, Path::new("test.yml"), &mut g1).unwrap();
        let b = build_inserts(&seed, &t, Path::new("test.yml"), &mut g2).unwrap();
        assert_eq!(a, b);
        // Identity column is skipped.
        assert!(a[0].sql.contains("([Name], [Age])"));
        assert_eq!(a[0].row_count, 3);
    }

    #[test]
    fn type_aware_picks_int_for_int_columns() {
        let mut g = Generator::from_seed(1);
        let v = g.type_aware(&col("X", "int", false));
        assert!(
            v.parse::<i64>().is_ok(),
            "expected integer literal, got {v}"
        );
    }

    #[test]
    fn type_aware_picks_bit_for_bit_column() {
        let mut g = Generator::from_seed(1);
        let v = g.type_aware(&col("X", "bit", false));
        assert!(v == "0" || v == "1");
    }

    #[test]
    fn type_aware_picks_uuid_for_uniqueidentifier() {
        let mut g = Generator::from_seed(1);
        let v = g.type_aware(&col("X", "uniqueidentifier", false));
        // 36 hex/dash chars + two surrounding quotes.
        assert_eq!(v.len(), 38);
        assert!(v.starts_with('\'') && v.ends_with('\''));
    }

    #[test]
    fn explicit_value_override_wins_over_faker() {
        let t = table("Foo", vec![col("Status", "nvarchar(20)", false)]);
        let mut cols = BTreeMap::new();
        cols.insert(
            "Status".into(),
            ColumnSpec {
                value: Some(serde_yaml::Value::String("active".into())),
                ..Default::default()
            },
        );
        let seed = SeedFile {
            table: "dbo.Foo".into(),
            rows: 2,
            columns: cols,
            batch_size: None,
        };
        let mut g = Generator::from_seed(1);
        let batches = build_inserts(&seed, &t, Path::new("x.yml"), &mut g).unwrap();
        assert!(batches[0].sql.contains("(N'active')"));
    }

    #[test]
    fn null_ratio_one_emits_null_for_nullable_column() {
        let t = table("Foo", vec![col("Note", "nvarchar(20)", true)]);
        let mut cols = BTreeMap::new();
        cols.insert(
            "Note".into(),
            ColumnSpec {
                null_ratio: Some(1.0),
                ..Default::default()
            },
        );
        let seed = SeedFile {
            table: "dbo.Foo".into(),
            rows: 2,
            columns: cols,
            batch_size: None,
        };
        let mut g = Generator::from_seed(1);
        let batches = build_inserts(&seed, &t, Path::new("x.yml"), &mut g).unwrap();
        assert!(batches[0].sql.contains("(NULL)"));
    }

    #[test]
    fn unknown_column_override_errors() {
        let t = table("Foo", vec![col("A", "int", false)]);
        let mut cols = BTreeMap::new();
        cols.insert("Bogus".into(), ColumnSpec::default());
        let seed = SeedFile {
            table: "dbo.Foo".into(),
            rows: 1,
            columns: cols,
            batch_size: None,
        };
        let mut g = Generator::from_seed(1);
        let err = build_inserts(&seed, &t, Path::new("x.yml"), &mut g).unwrap_err();
        matches!(err, SeedError::UnknownColumn { .. });
    }

    #[test]
    fn batch_size_splits_rows() {
        let t = table("Foo", vec![col("A", "int", false)]);
        let seed = SeedFile {
            table: "dbo.Foo".into(),
            rows: 5,
            columns: BTreeMap::new(),
            batch_size: Some(2),
        };
        let mut g = Generator::from_seed(1);
        let batches = build_inserts(&seed, &t, Path::new("x.yml"), &mut g).unwrap();
        assert_eq!(batches.len(), 3);
        assert_eq!(batches[0].row_count, 2);
        assert_eq!(batches[1].row_count, 2);
        assert_eq!(batches[2].row_count, 1);
    }
}
