//! Output formatting for `sqldev query`.
//!
//! M1.4 introduces a small typed-value layer ([`CellValue`]) between the
//! Tiberius row and the final formatter. Each formatter (text TSV,
//! aligned table, CSV, JSON array, NDJSON) renders a [`RowSet`] that is
//! independent of the Tiberius types.
//!
//! Today we extract the same value kinds the previous string-only path
//! supported: `&str`, `i64`, `i32`, `i16`, `bool`, `f64`, and NULL. The
//! shape of [`CellValue`] is intentionally an open enum so that decimal,
//! datetime, UUID, and binary variants can be added in M1.5 without
//! touching any formatter.

pub mod csv_fmt;
pub mod json_fmt;
pub mod ndjson_fmt;
pub mod table_fmt;
pub mod text_fmt;
pub mod value;

pub use value::{CellValue, RowSet};
