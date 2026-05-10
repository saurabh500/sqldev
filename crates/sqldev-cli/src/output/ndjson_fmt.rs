//! Newline-delimited JSON: one row object per line. Stream-friendly.

use std::io::Write;

use anyhow::{Context, Result};
use serde_json::{Map, Value};

use super::RowSet;

pub fn write<W: Write>(rs: &RowSet, mut w: W) -> Result<()> {
    for row in &rs.rows {
        let mut obj = Map::with_capacity(rs.columns.len());
        for (i, col) in rs.columns.iter().enumerate() {
            let cell = row.get(i).map_or(Value::Null, super::CellValue::to_json);
            obj.insert(col.clone(), cell);
        }
        let line = serde_json::to_string(&Value::Object(obj)).context("serialize NDJSON line")?;
        writeln!(w, "{line}").context("write NDJSON line")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::CellValue;
    use super::*;

    #[test]
    fn one_object_per_line() {
        let mut rs = RowSet::new(vec!["n".into()]);
        rs.push(vec![CellValue::Int(1)]);
        rs.push(vec![CellValue::Int(2)]);
        let mut buf = Vec::new();
        write(&rs, &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        let lines: Vec<&str> = s.lines().collect();
        assert_eq!(lines.len(), 2);
        let first: Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(first["n"], Value::Number(1.into()));
    }
}
