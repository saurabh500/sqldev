//! Typed JSON-array output. One JSON array of row objects; each cell is
//! rendered with [`CellValue::to_json`] so numbers / booleans / NULLs
//! preserve type fidelity.

use std::io::Write;

use anyhow::{Context, Result};
use serde_json::{Map, Value};

use super::RowSet;

pub fn write<W: Write>(rs: &RowSet, mut w: W) -> Result<()> {
    let mut out: Vec<Value> = Vec::with_capacity(rs.rows.len());
    for row in &rs.rows {
        let mut obj = Map::with_capacity(rs.columns.len());
        for (i, col) in rs.columns.iter().enumerate() {
            let cell = row.get(i).map_or(Value::Null, super::CellValue::to_json);
            obj.insert(col.clone(), cell);
        }
        out.push(Value::Object(obj));
    }
    let s = serde_json::to_string(&Value::Array(out)).context("serialize JSON")?;
    writeln!(w, "{s}").context("write JSON")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::CellValue;
    use super::*;

    #[test]
    fn renders_typed_values() {
        let mut rs = RowSet::new(vec!["n".into(), "ok".into(), "name".into()]);
        rs.push(vec![
            CellValue::Int(1),
            CellValue::Bool(true),
            CellValue::Null,
        ]);
        let mut buf = Vec::new();
        write(&rs, &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        let parsed: Value = serde_json::from_str(s.trim()).unwrap();
        assert_eq!(parsed[0]["n"], Value::Number(1.into()));
        assert_eq!(parsed[0]["ok"], Value::Bool(true));
        assert_eq!(parsed[0]["name"], Value::Null);
    }
}
