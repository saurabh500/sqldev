// sqldev M0.3 spike: showplan XML anti-pattern detector.
//
// Reads SQL Server showplan XML on stdin or from --plan FILE and emits one
// human-readable suggestion per anti-pattern found. Targets the three
// patterns called out in the M0 plan:
//
//   1. Full table / clustered-index scan with a residual predicate
//      (i.e. the optimizer scanned the whole heap because no index was
//      useful for the WHERE clause).
//   2. Key Lookup / RID Lookup (selected non-covering index forced a
//      bookmark lookup).
//   3. Implicit conversion in a predicate (often blocks index seeks).
//
// The spike only needs to find these in real AdventureWorks plans; it
// doesn't need to emit fix SQL — that comes in M2.

use anyhow::{Context, Result};
use clap::Parser;
use roxmltree::{Document, Node};
use std::fs;
use std::io::{self, Read};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(version, about = "sqldev showplan anti-pattern spike")]
struct Args {
    /// File containing showplan XML. If omitted, read from stdin.
    #[arg(long)]
    plan: Option<PathBuf>,
}

#[derive(Debug)]
struct Finding {
    rule: &'static str,
    severity: &'static str,
    statement_text: Option<String>,
    target: Option<String>, // table or column reference
    message: String,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let xml = match args.plan {
        Some(p) => fs::read_to_string(&p).with_context(|| format!("reading {}", p.display()))?,
        None => {
            let mut s = String::new();
            io::stdin().read_to_string(&mut s)?;
            s
        }
    };

    // sqlcmd sometimes intermixes the rowset with the plan; trim to the
    // <ShowPlanXML ... > ... </ShowPlanXML> envelope.
    let xml = isolate_showplan(&xml);

    let doc = Document::parse(&xml).context("parse showplan XML")?;
    let mut findings: Vec<Finding> = vec![];

    for stmt in doc.descendants().filter(|n| n.has_tag_name("StmtSimple")) {
        let stmt_text = stmt.attribute("StatementText").map(|s| s.to_string());
        for relop in stmt.descendants().filter(|n| n.has_tag_name("RelOp")) {
            inspect_relop(relop, &stmt_text, &mut findings);
        }
    }

    if findings.is_empty() {
        println!("plan looks clean (no scan/lookup/implicit-conversion anti-patterns detected)");
        return Ok(());
    }
    for f in &findings {
        println!("[{}] {}", f.severity, f.rule);
        if let Some(s) = &f.statement_text {
            println!("    statement: {}", truncate(s, 100));
        }
        if let Some(t) = &f.target {
            println!("    target:    {}", t);
        }
        println!("    finding:   {}", f.message);
    }
    println!();
    println!("{} finding(s)", findings.len());
    Ok(())
}

fn inspect_relop(relop: Node, stmt_text: &Option<String>, out: &mut Vec<Finding>) {
    let physical = relop.attribute("PhysicalOp").unwrap_or("");
    let est_rows: f64 = relop.attribute("EstimateRows").and_then(|s| s.parse().ok()).unwrap_or(0.0);
    let card: f64 = relop.attribute("TableCardinality").and_then(|s| s.parse().ok()).unwrap_or(0.0);

    // 1. Table / clustered-index scan with no seek predicate.
    let is_scan = matches!(
        physical,
        "Table Scan" | "Clustered Index Scan" | "Index Scan"
    );
    if is_scan {
        let has_seek = relop.descendants().any(|n| n.has_tag_name("SeekPredicates") || n.has_tag_name("SeekPredicate"));
        let has_residual = relop
            .children()
            .filter(|n| n.is_element())
            .any(|n| n.children().any(|c| c.has_tag_name("Predicate")))
            || relop.descendants().any(|n| {
                n.has_tag_name("Predicate")
                    && n.parent()
                        .map(|p| p.has_tag_name("TableScan") || p.has_tag_name("IndexScan"))
                        .unwrap_or(false)
            });
        // Skip tiny tables (<100 rows) — scans there are fine.
        let big_enough = card >= 100.0 || card == 0.0;
        if !has_seek && (has_residual || physical == "Table Scan") && big_enough {
            let target = scan_object(relop);
            let msg = if physical == "Table Scan" {
                format!(
                    "heap table is being scanned; consider adding a clustered index or a nonclustered index that covers the WHERE/JOIN predicate (cardinality ~{})",
                    card as i64
                )
            } else {
                format!(
                    "{} returns {} of ~{} rows but the predicate could not be pushed into a seek; consider an index covering the WHERE/JOIN predicate",
                    physical, est_rows as i64, card as i64
                )
            };
            out.push(Finding {
                rule: "scan-with-residual-predicate",
                severity: "warning",
                statement_text: stmt_text.clone(),
                target,
                message: msg,
            });
        }
    }

    // 2. Key / RID lookup. SQL Server emits these as `Clustered Index Seek`
    // (or `RID Lookup`) with an inner `<IndexScan Lookup="1">`. The legacy
    // `PhysicalOp="Key Lookup"` form also exists in older plans.
    let is_lookup = physical == "Key Lookup"
        || physical == "RID Lookup"
        || (physical == "Clustered Index Seek"
            && relop
                .descendants()
                .any(|n| n.has_tag_name("IndexScan") && n.attribute("Lookup") == Some("1")));
    if is_lookup {
        // Find the *outer* index whose row produced this lookup. The plan
        // shape is: NestedLoops > IndexScan/IndexSeek (outer) + KeyLookup (inner).
        let outer_index = relop
            .ancestors()
            .find(|n| n.has_tag_name("RelOp"))
            .and_then(|nl| {
                nl.descendants()
                    .filter(|n| {
                        n.has_tag_name("RelOp")
                            && n.attribute("NodeId") != relop.attribute("NodeId")
                            && matches!(
                                n.attribute("PhysicalOp").unwrap_or(""),
                                "Index Seek" | "Index Scan"
                            )
                    })
                    .find_map(|seek| {
                        seek.descendants()
                            .find(|n| n.has_tag_name("Object"))
                            .and_then(|o| o.attribute("Index").map(|s| s.to_string()))
                    })
            });
        let target = scan_object(relop);
        let msg = match outer_index {
            Some(idx) => format!(
                "non-covering index {idx} forces a {physical}; consider INCLUDE-ing the SELECTed columns in {idx}"
            ),
            None => format!("{physical} present; the chosen index does not cover the SELECT list"),
        };
        out.push(Finding {
            rule: "non-covering-index-lookup",
            severity: "warning",
            statement_text: stmt_text.clone(),
            target,
            message: msg,
        });
    }

    // 3. Implicit conversion. Only flag when the Convert is inside a
    // <Predicate> (the seekable-predicate context). Implicit converts in
    // <Compute Scalar> / <DefinedValue> are computed-column or projection
    // expressions and do not affect index seek-ability.
    for conv in relop.descendants().filter(|n| n.has_tag_name("Convert")) {
        if conv.attribute("Implicit") != Some("1") {
            continue;
        }
        let in_predicate = conv
            .ancestors()
            .any(|a| a.has_tag_name("Predicate") || a.has_tag_name("SeekPredicate") || a.has_tag_name("SeekPredicateNew"));
        if !in_predicate {
            continue;
        }
        let on_column = conv
            .descendants()
            .any(|n| n.has_tag_name("ColumnReference") && n.attribute("Column").is_some());
        if !on_column {
            continue;
        }
        let col = conv
            .descendants()
            .find(|n| n.has_tag_name("ColumnReference"))
            .map(|c| {
                let s = c.attribute("Schema").unwrap_or("");
                let t = c.attribute("Table").unwrap_or("");
                let n = c.attribute("Column").unwrap_or("");
                format!("{s}.{t}.{n}")
            });
        let from = conv.attribute("DataType").unwrap_or("?");
        out.push(Finding {
            rule: "implicit-conversion-on-column",
            severity: "warning",
            statement_text: stmt_text.clone(),
            target: col,
            message: format!(
                "implicit conversion to {from} on a column reference inside a predicate; the optimizer cannot seek the underlying index. Match the parameter type to the column type."
            ),
        });
    }
}

/// Pull the `<Object Database=… Schema=… Table=… Index=…/>` reference if any.
fn scan_object(relop: Node) -> Option<String> {
    relop.descendants().find(|n| n.has_tag_name("Object")).map(|o| {
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

fn isolate_showplan(s: &str) -> String {
    let start = s.find("<ShowPlanXML");
    let end = s.find("</ShowPlanXML>");
    match (start, end) {
        (Some(a), Some(b)) => s[a..b + "</ShowPlanXML>".len()].to_string(),
        _ => s.to_string(),
    }
}

fn truncate(s: &str, n: usize) -> String {
    let s = s.trim();
    if s.chars().count() <= n {
        s.to_string()
    } else {
        s.chars().take(n).collect::<String>() + "…"
    }
}
