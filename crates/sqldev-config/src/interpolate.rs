//! Env-var interpolation for `.sqldev.yml`.
//!
//! Supported syntax (matches the docker-compose / shell convention):
//! - `${VAR}` — required; errors if `VAR` is not set
//! - `${VAR:-fallback}` — uses `VAR` if set and non-empty, else `fallback`
//! - `$$` — literal `$` (escape)
//!
//! Anything else starting with `$` is left as-is (passed through untouched)
//! so YAML strings such as `"$id"` or jq expressions don't break.

use std::collections::HashMap;

#[derive(Debug, thiserror::Error)]
pub enum InterpolateError {
    #[error("environment variable `{0}` referenced in config is not set")]
    Missing(String),
    #[error("malformed `${{...}}` reference in config (unterminated brace)")]
    Unterminated,
    #[error("invalid env-var name `{0}` in config; names must match [A-Za-z_][A-Za-z0-9_]*")]
    InvalidName(String),
}

/// Interpolate `${VAR}` and `${VAR:-fallback}` against the process env.
///
/// `extra` is consulted before the process environment, so callers can
/// inject overrides (useful in tests).
pub fn interpolate(
    input: &str,
    extra: &HashMap<String, String>,
) -> Result<String, InterpolateError> {
    let mut out = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'$' {
            out.push(bytes[i] as char);
            i += 1;
            continue;
        }
        // We're at a `$`. Look ahead.
        if i + 1 >= bytes.len() {
            out.push('$');
            i += 1;
            continue;
        }
        let next = bytes[i + 1];
        if next == b'$' {
            // Escape: `$$` -> `$`.
            out.push('$');
            i += 2;
            continue;
        }
        if next != b'{' {
            // Not our syntax; pass through.
            out.push('$');
            i += 1;
            continue;
        }
        // Find matching `}`.
        let close = match bytes[i + 2..].iter().position(|&b| b == b'}') {
            Some(p) => i + 2 + p,
            None => return Err(InterpolateError::Unterminated),
        };
        let body = &input[i + 2..close];
        let (name, fallback) = match body.split_once(":-") {
            Some((n, f)) => (n, Some(f)),
            None => (body, None),
        };
        if !is_valid_env_name(name) {
            return Err(InterpolateError::InvalidName(name.to_string()));
        }
        let value = if let Some(v) = extra.get(name) {
            v.clone()
        } else {
            std::env::var(name).ok().unwrap_or_default()
        };
        if value.is_empty() {
            match fallback {
                Some(f) => out.push_str(f),
                None => return Err(InterpolateError::Missing(name.to_string())),
            }
        } else {
            out.push_str(&value);
        }
        i = close + 1;
    }
    Ok(out)
}

fn is_valid_env_name(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn extra(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
    }

    #[test]
    fn passthrough_no_dollars() {
        let out = interpolate("hello world", &HashMap::new()).unwrap();
        assert_eq!(out, "hello world");
    }

    #[test]
    fn simple_substitution() {
        let out = interpolate("p=${PASS}", &extra(&[("PASS", "s3cret")])).unwrap();
        assert_eq!(out, "p=s3cret");
    }

    #[test]
    fn fallback_when_unset() {
        let out = interpolate("h=${MISSING:-localhost}", &HashMap::new()).unwrap();
        assert_eq!(out, "h=localhost");
    }

    #[test]
    fn fallback_skipped_when_set() {
        let out = interpolate("h=${H:-localhost}", &extra(&[("H", "db.example.com")])).unwrap();
        assert_eq!(out, "h=db.example.com");
    }

    #[test]
    fn missing_without_fallback_is_error() {
        let err = interpolate("p=${ABSENT_VAR_XYZ}", &HashMap::new()).unwrap_err();
        matches!(err, InterpolateError::Missing(_));
    }

    #[test]
    fn dollar_escape() {
        let out = interpolate("price=$$5", &HashMap::new()).unwrap();
        assert_eq!(out, "price=$5");
    }

    #[test]
    fn unrelated_dollar_passes_through() {
        let out = interpolate("$id and $1", &HashMap::new()).unwrap();
        assert_eq!(out, "$id and $1");
    }

    #[test]
    fn unterminated_brace_errors() {
        assert!(interpolate("${UNCLOSED", &HashMap::new()).is_err());
    }

    #[test]
    fn invalid_name_errors() {
        assert!(interpolate("${1BAD}", &HashMap::new()).is_err());
    }
}
