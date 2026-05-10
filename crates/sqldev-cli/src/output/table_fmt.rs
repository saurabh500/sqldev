//! Aligned text-table output, suitable for human-readable terminal display.
//!
//! Columns are sized to the widest cell (header included). Numeric columns
//! are right-aligned; everything else is left-aligned. NULLs render as the
//! literal `NULL` so they're distinguishable from empty strings.

use std::io::Write;

use anyhow::{Context, Result};

use super::{CellValue, RowSet};

const NULL_MARKER: &str = "NULL";

pub fn write<W: Write>(rs: &RowSet, mut w: W) -> Result<()> {
    if rs.is_empty() {
        return Ok(());
    }
    let cells = render_cells(rs);
    let widths = column_widths(&rs.columns, &cells);
    let numeric = numeric_columns(rs);

    write_row(&mut w, &rs.columns, &widths, &numeric)?;
    write_separator(&mut w, &widths)?;
    for row in &cells {
        write_row(&mut w, row, &widths, &numeric)?;
    }
    Ok(())
}

fn render_cells(rs: &RowSet) -> Vec<Vec<String>> {
    rs.rows
        .iter()
        .map(|row| {
            row.iter()
                .map(|c| {
                    if c.is_null() {
                        NULL_MARKER.to_string()
                    } else {
                        c.to_plain()
                    }
                })
                .collect()
        })
        .collect()
}

fn column_widths(headers: &[String], rows: &[Vec<String>]) -> Vec<usize> {
    let mut widths: Vec<usize> = headers.iter().map(|h| h.chars().count()).collect();
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            if i < widths.len() {
                widths[i] = widths[i].max(cell.chars().count());
            }
        }
    }
    widths
}

fn numeric_columns(rs: &RowSet) -> Vec<bool> {
    let n = rs.columns.len();
    let mut numeric = vec![true; n];
    for row in &rs.rows {
        for (i, c) in row.iter().enumerate() {
            if i < n && !is_numeric(c) {
                numeric[i] = false;
            }
        }
    }
    numeric
}

fn is_numeric(c: &CellValue) -> bool {
    matches!(
        c,
        CellValue::BigInt(_)
            | CellValue::Int(_)
            | CellValue::SmallInt(_)
            | CellValue::Float(_)
            | CellValue::Null
    )
}

fn write_row<W: Write>(
    w: &mut W,
    cells: &[String],
    widths: &[usize],
    numeric: &[bool],
) -> Result<()> {
    let mut parts = Vec::with_capacity(cells.len());
    for (i, cell) in cells.iter().enumerate() {
        let width = widths.get(i).copied().unwrap_or(0);
        let right_align = numeric.get(i).copied().unwrap_or(false);
        if right_align {
            parts.push(format!("{cell:>width$}"));
        } else {
            parts.push(format!("{cell:<width$}"));
        }
    }
    writeln!(w, "{}", parts.join("  ")).context("write table row")
}

fn write_separator<W: Write>(w: &mut W, widths: &[usize]) -> Result<()> {
    let parts: Vec<String> = widths.iter().map(|n| "-".repeat(*n)).collect();
    writeln!(w, "{}", parts.join("  ")).context("write table separator")
}

#[cfg(test)]
mod tests {
    use super::super::CellValue;
    use super::*;

    #[test]
    fn aligns_text_left_and_numbers_right() {
        let mut rs = RowSet::new(vec!["name".into(), "id".into()]);
        rs.push(vec![CellValue::String("alpha".into()), CellValue::Int(1)]);
        rs.push(vec![CellValue::String("b".into()), CellValue::Int(123)]);
        let mut buf = Vec::new();
        write(&rs, &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        // header + separator + 2 rows
        let lines: Vec<&str> = s.lines().collect();
        assert_eq!(lines.len(), 4);
        assert_eq!(lines[0], "name    id");
        assert_eq!(lines[1], "-----  ---");
        assert_eq!(lines[2], "alpha    1");
        assert_eq!(lines[3], "b      123");
    }

    #[test]
    fn null_is_visible() {
        let mut rs = RowSet::new(vec!["x".into()]);
        rs.push(vec![CellValue::Null]);
        let mut buf = Vec::new();
        write(&rs, &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(s.contains("NULL"), "expected NULL marker, got: {s:?}");
    }

    #[test]
    fn empty_rowset_writes_nothing() {
        let mut buf = Vec::new();
        write(&RowSet::default(), &mut buf).unwrap();
        assert!(buf.is_empty());
    }
}
