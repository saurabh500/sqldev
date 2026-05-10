//! Tab-separated text output. Default and pipe-friendly.

use std::io::Write;

use anyhow::{Context, Result};

use super::RowSet;

pub fn write<W: Write>(rs: &RowSet, mut w: W) -> Result<()> {
    if rs.is_empty() {
        return Ok(());
    }
    writeln!(w, "{}", rs.columns.join("\t")).context("write header")?;
    for row in &rs.rows {
        let cells: Vec<String> = row.iter().map(super::CellValue::to_plain).collect();
        writeln!(w, "{}", cells.join("\t")).context("write row")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::CellValue;
    use super::*;

    #[test]
    fn empty_rowset_writes_nothing() {
        let rs = RowSet::default();
        let mut buf = Vec::new();
        write(&rs, &mut buf).unwrap();
        assert!(buf.is_empty());
    }

    #[test]
    fn header_then_rows_tab_separated() {
        let mut rs = RowSet::new(vec!["a".into(), "b".into()]);
        rs.push(vec![CellValue::Int(1), CellValue::String("x".into())]);
        rs.push(vec![CellValue::Null, CellValue::Bool(true)]);
        let mut buf = Vec::new();
        write(&rs, &mut buf).unwrap();
        assert_eq!(String::from_utf8(buf).unwrap(), "a\tb\n1\tx\n\ttrue\n");
    }
}
