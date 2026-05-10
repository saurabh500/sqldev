//! RFC 4180 CSV output (via the `csv` crate).

use std::io::Write;

use anyhow::{Context, Result};

use super::RowSet;

pub fn write<W: Write>(rs: &RowSet, w: W) -> Result<()> {
    let mut wtr = csv::WriterBuilder::new().from_writer(w);
    if !rs.columns.is_empty() {
        wtr.write_record(rs.columns.iter().map(String::as_str))
            .context("write CSV header")?;
    }
    for row in &rs.rows {
        let cells: Vec<String> = row.iter().map(super::CellValue::to_plain).collect();
        wtr.write_record(cells.iter().map(String::as_str))
            .context("write CSV row")?;
    }
    wtr.flush().context("flush CSV writer")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::CellValue;
    use super::*;

    #[test]
    fn quotes_values_with_special_chars() {
        let mut rs = RowSet::new(vec!["a".into(), "b".into()]);
        rs.push(vec![
            CellValue::String("with,comma".into()),
            CellValue::String("with\"quote".into()),
        ]);
        rs.push(vec![CellValue::Null, CellValue::Int(42)]);
        let mut buf = Vec::new();
        write(&rs, &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert_eq!(s, "a,b\n\"with,comma\",\"with\"\"quote\"\n,42\n");
    }

    #[test]
    fn empty_rowset_writes_nothing() {
        let mut buf = Vec::new();
        write(&RowSet::default(), &mut buf).unwrap();
        assert!(buf.is_empty());
    }
}
