//! Showplan XML analyzer.
//!
//! Parses a SQL Server `<ShowPlanXML>` document and surfaces common
//! query-tuning anti-patterns ("scans where seeks should be", "key
//! lookups on non-covering indexes", "implicit conversions blocking
//! index use", and so on).
//!
//! The catalog targets the ~15 rules called out in the M2.1 issue.
//! Each rule emits a [`Finding`] with:
//!
//! * `rule` — a kebab-case identifier suitable for filtering
//! * `severity` — `info` / `warning` / `error`
//! * `statement_text` — the offending statement (best-effort)
//! * `target` — schema-qualified table or column reference
//! * `message` — a human-readable description + suggested fix
//! * `est_cost_delta` — when computable, an estimate of how much
//!   subtree cost the fix would save (heuristic, not a guarantee)
//!
//! This crate is intentionally I/O free. The CLI wraps it.

#![forbid(unsafe_code)]

use anyhow::{Context, Result};
use roxmltree::{Document, Node};
use serde::Serialize;

mod rules;

/// Severity of a finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Info,
    Warning,
    Error,
}

impl Severity {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Warning => "warning",
            Self::Error => "error",
        }
    }
}

/// A single anti-pattern finding.
#[derive(Debug, Clone, Serialize)]
pub struct Finding {
    pub rule: &'static str,
    pub severity: Severity,
    pub statement_text: Option<String>,
    pub target: Option<String>,
    pub message: String,
    /// Estimated cost units that the suggested fix could save. `None`
    /// when not computable from the plan alone.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub est_cost_delta: Option<f64>,
}

/// Parse a Showplan XML document and run every rule against it.
///
/// Accepts either a bare `<ShowPlanXML>` document or a sqlcmd transcript
/// that happens to contain one (the analyzer trims to the
/// `<ShowPlanXML>…</ShowPlanXML>` envelope before parsing).
pub fn analyze_plan(xml: &str) -> Result<Vec<Finding>> {
    let isolated = isolate_showplan(xml);
    let doc = Document::parse(&isolated).context("parse showplan XML")?;
    let mut out: Vec<Finding> = Vec::new();
    for stmt in doc.descendants().filter(|n| n.has_tag_name("StmtSimple")) {
        let stmt_text = stmt.attribute("StatementText").map(str::to_string);
        rules::run_statement_rules(stmt, stmt_text.as_ref(), &mut out);
        for relop in stmt.descendants().filter(|n| n.has_tag_name("RelOp")) {
            rules::run_relop_rules(relop, stmt_text.as_ref(), &mut out);
        }
    }
    Ok(out)
}

/// Pulls the `<Object Database=… Schema=… Table=… Index=…/>` reference
/// closest to `relop`. Public so rules in the `rules` module can share.
pub(crate) fn scan_object(relop: Node) -> Option<String> {
    relop
        .descendants()
        .find(|n| n.has_tag_name("Object"))
        .map(|o| {
            let s = o.attribute("Schema").unwrap_or("").trim_matches(['[', ']']);
            let t = o.attribute("Table").unwrap_or("").trim_matches(['[', ']']);
            let i = o.attribute("Index").unwrap_or("").trim_matches(['[', ']']);
            if i.is_empty() {
                format!("{s}.{t}")
            } else {
                format!("{s}.{t} ({i})")
            }
        })
}

pub(crate) fn estimate_rows(n: Node) -> f64 {
    n.attribute("EstimateRows")
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(0.0)
}

pub(crate) fn estimate_cost(n: Node) -> f64 {
    n.attribute("EstimatedTotalSubtreeCost")
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(0.0)
}

pub(crate) fn table_cardinality(n: Node) -> f64 {
    n.attribute("TableCardinality")
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(0.0)
}

fn isolate_showplan(s: &str) -> String {
    let start = s.find("<ShowPlanXML");
    let end = s.find("</ShowPlanXML>");
    match (start, end) {
        (Some(a), Some(b)) => s[a..b + "</ShowPlanXML>".len()].to_string(),
        _ => s.to_string(),
    }
}
