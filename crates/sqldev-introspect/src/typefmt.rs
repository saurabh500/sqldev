//! SQL Server type / default formatting helpers.

/// Format a SQL Server type name with length/precision/scale, matching the
/// shape developers actually write in CREATE TABLE statements.
pub fn format_type(base: &str, max_length: i16, precision: u8, scale: u8) -> String {
    let b = base.to_ascii_lowercase();
    match b.as_str() {
        "varchar" | "char" | "varbinary" | "binary" => {
            if max_length == -1 {
                format!("{b}(max)")
            } else {
                format!("{b}({max_length})")
            }
        }
        "nvarchar" | "nchar" => {
            if max_length == -1 {
                format!("{b}(max)")
            } else {
                // n-char max_length is bytes; halve for char count.
                format!("{b}({})", max_length / 2)
            }
        }
        "decimal" | "numeric" => format!("{b}({precision},{scale})"),
        "datetime2" | "datetimeoffset" | "time" => {
            if scale == 7 {
                b
            } else {
                format!("{b}({scale})")
            }
        }
        "float" => {
            if precision == 53 {
                "float".into()
            } else {
                format!("float({precision})")
            }
        }
        _ => b,
    }
}

/// SQL Server wraps default expressions in extra parens (e.g. `((0))`).
/// Strip balanced outer pairs for readability.
pub fn strip_default_wrap(s: &str) -> String {
    let mut t = s.trim().to_string();
    while t.len() >= 2 && t.starts_with('(') && t.ends_with(')') {
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
