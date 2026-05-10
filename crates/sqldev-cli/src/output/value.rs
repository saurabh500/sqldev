//! Typed cell values and row sets.
//!
//! [`CellValue`] losslessly captures every value kind the CLI extracts
//! from a Tiberius row today. It is deliberately an open enum so we can
//! add decimal, datetime, UUID, and binary variants in later milestones
//! without changing any formatter signature.

use serde_json::{Number, Value as JsonValue};

/// One cell of a query result.
#[derive(Clone, Debug, PartialEq)]
pub enum CellValue {
    Null,
    String(String),
    BigInt(i64),
    Int(i32),
    SmallInt(i16),
    Float(f64),
    Bool(bool),
}

impl CellValue {
    /// Plain-text rendering used by TSV / aligned-table / CSV cells.
    /// NULL renders as the empty string. Callers that need a visible
    /// marker (e.g. the aligned table formatter) substitute their own.
    pub fn to_plain(&self) -> String {
        match self {
            Self::Null => String::new(),
            Self::String(s) => s.clone(),
            Self::BigInt(v) => v.to_string(),
            Self::Int(v) => v.to_string(),
            Self::SmallInt(v) => v.to_string(),
            Self::Float(v) => v.to_string(),
            Self::Bool(v) => v.to_string(),
        }
    }

    /// Typed JSON rendering. Numbers stay numbers, booleans stay
    /// booleans, NULL becomes `Value::Null`. Floats that aren't finite
    /// (NaN / +Inf / -Inf) fall back to `Value::Null` because RFC 8259
    /// JSON cannot represent them.
    pub fn to_json(&self) -> JsonValue {
        match self {
            Self::Null => JsonValue::Null,
            Self::String(s) => JsonValue::String(s.clone()),
            Self::BigInt(v) => JsonValue::Number((*v).into()),
            Self::Int(v) => JsonValue::Number(i64::from(*v).into()),
            Self::SmallInt(v) => JsonValue::Number(i64::from(*v).into()),
            Self::Float(v) => Number::from_f64(*v).map_or(JsonValue::Null, JsonValue::Number),
            Self::Bool(v) => JsonValue::Bool(*v),
        }
    }

    /// Whether this cell is NULL. The aligned-table formatter uses this
    /// to substitute a visible `NULL` marker instead of empty cells.
    pub fn is_null(&self) -> bool {
        matches!(self, Self::Null)
    }
}

/// A complete result set, formatter-agnostic.
#[derive(Clone, Debug, Default)]
pub struct RowSet {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<CellValue>>,
}

impl RowSet {
    pub fn new(columns: Vec<String>) -> Self {
        Self {
            columns,
            rows: Vec::new(),
        }
    }

    pub fn push(&mut self, row: Vec<CellValue>) {
        self.rows.push(row);
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }
}

/// Extract a typed [`RowSet`] from a slice of Tiberius rows. Mirrors the
/// type fall-through the M1.3 implementation used (`&str`, `i64`, `i32`,
/// `i16`, `bool`, `f64`) but wraps each successful read in a typed
/// [`CellValue`] instead of stringifying.
pub fn extract(rows: &[mssql_tiberius_bridge::Row]) -> RowSet {
    if rows.is_empty() {
        return RowSet::default();
    }
    let columns: Vec<String> = rows[0]
        .columns()
        .iter()
        .map(|c| c.name().to_string())
        .collect();
    let mut out = RowSet::new(columns.clone());
    for r in rows {
        let mut cells = Vec::with_capacity(columns.len());
        for i in 0..columns.len() {
            cells.push(extract_cell(r, i));
        }
        out.push(cells);
    }
    out
}

fn extract_cell(row: &mssql_tiberius_bridge::Row, idx: usize) -> CellValue {
    if let Some(v) = row.get::<&str, _>(idx) {
        return CellValue::String(v.to_string());
    }
    if let Some(v) = row.get::<i64, _>(idx) {
        return CellValue::BigInt(v);
    }
    if let Some(v) = row.get::<i32, _>(idx) {
        return CellValue::Int(v);
    }
    if let Some(v) = row.get::<i16, _>(idx) {
        return CellValue::SmallInt(v);
    }
    if let Some(v) = row.get::<bool, _>(idx) {
        return CellValue::Bool(v);
    }
    if let Some(v) = row.get::<f64, _>(idx) {
        return CellValue::Float(v);
    }
    CellValue::Null
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_renders_each_variant() {
        assert_eq!(CellValue::Null.to_plain(), "");
        assert_eq!(CellValue::String("hi".into()).to_plain(), "hi");
        assert_eq!(CellValue::BigInt(7).to_plain(), "7");
        assert_eq!(CellValue::Int(-3).to_plain(), "-3");
        assert_eq!(CellValue::SmallInt(2).to_plain(), "2");
        assert_eq!(CellValue::Bool(true).to_plain(), "true");
        assert_eq!(CellValue::Float(1.5).to_plain(), "1.5");
    }

    #[test]
    fn json_preserves_types() {
        assert_eq!(CellValue::Null.to_json(), JsonValue::Null);
        assert_eq!(
            CellValue::String("hi".into()).to_json(),
            JsonValue::String("hi".into())
        );
        assert!(CellValue::BigInt(42).to_json().is_i64());
        assert!(CellValue::Int(42).to_json().is_i64());
        assert!(CellValue::Bool(false).to_json() == JsonValue::Bool(false));
        assert!(CellValue::Float(1.5).to_json().is_f64());
    }

    #[test]
    fn json_non_finite_floats_become_null() {
        assert_eq!(CellValue::Float(f64::NAN).to_json(), JsonValue::Null);
        assert_eq!(CellValue::Float(f64::INFINITY).to_json(), JsonValue::Null);
        assert_eq!(
            CellValue::Float(f64::NEG_INFINITY).to_json(),
            JsonValue::Null
        );
    }

    #[test]
    fn rowset_basics() {
        let mut rs = RowSet::new(vec!["a".into()]);
        assert!(rs.is_empty());
        rs.push(vec![CellValue::Int(1)]);
        assert_eq!(rs.len(), 1);
        assert!(!rs.is_empty());
    }
}
