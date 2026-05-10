//! Anti-pattern rule catalog.
//!
//! Rules are split into two phases:
//! * [`run_statement_rules`] runs once per `<StmtSimple>` (statement-
//!   wide signals — missing indexes, query-level warnings, etc.).
//! * [`run_relop_rules`] runs for every `<RelOp>` in the tree
//!   (operator-shape signals — scans, lookups, spools, joins).

use roxmltree::Node;

use crate::{Finding, Severity, estimate_cost, estimate_rows, scan_object, table_cardinality};

pub(crate) fn run_statement_rules(stmt: Node, stmt_text: Option<&String>, out: &mut Vec<Finding>) {
    rule_missing_index_hint(stmt, stmt_text, out);
    rule_parallelism_cost_warning(stmt, stmt_text, out);
    rule_join_without_statistics(stmt, stmt_text, out);
    rule_excessive_memory_grant(stmt, stmt_text, out);
}

pub(crate) fn run_relop_rules(relop: Node, stmt_text: Option<&String>, out: &mut Vec<Finding>) {
    rule_scan_with_residual(relop, stmt_text, out);
    rule_non_covering_lookup(relop, stmt_text, out);
    rule_implicit_conversion(relop, stmt_text, out);
    rule_hash_spill(relop, stmt_text, out);
    rule_sort_warning(relop, stmt_text, out);
    rule_parameter_sniffing(relop, stmt_text, out);
    rule_nested_loops_large_outer(relop, stmt_text, out);
    rule_eager_spool(relop, stmt_text, out);
    rule_udf_in_predicate(relop, stmt_text, out);
}

// ---------------------------------------------------------------------------
// Statement-wide rules
// ---------------------------------------------------------------------------

/// `<MissingIndexes>` group attached to a statement is the optimizer
/// literally telling us what index would help. Surface it verbatim.
fn rule_missing_index_hint(stmt: Node, stmt_text: Option<&String>, out: &mut Vec<Finding>) {
    for grp in stmt
        .descendants()
        .filter(|n| n.has_tag_name("MissingIndexGroup"))
    {
        let impact: f64 = grp
            .attribute("Impact")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0.0);
        let mi = grp.descendants().find(|n| n.has_tag_name("MissingIndex"));
        let target = mi.and_then(|m| {
            let s = m.attribute("Schema").unwrap_or("").trim_matches(['[', ']']);
            let t = m.attribute("Table").unwrap_or("").trim_matches(['[', ']']);
            (!t.is_empty()).then(|| format!("{s}.{t}"))
        });
        let cols: Vec<String> = mi
            .map(|m| {
                m.descendants()
                    .filter(|n| n.has_tag_name("Column"))
                    .filter_map(|n| {
                        n.attribute("Name")
                            .map(|s| s.trim_matches(['[', ']']).to_string())
                    })
                    .collect()
            })
            .unwrap_or_default();
        let cols_str = if cols.is_empty() {
            "(see plan)".to_string()
        } else {
            cols.join(", ")
        };
        out.push(Finding {
            rule: "missing-index-hint",
            severity: Severity::Warning,
            statement_text: stmt_text.cloned(),
            target,
            message: format!(
                "optimizer reports a missing index would help (impact ~{impact:.0}%); candidate columns: {cols_str}"
            ),
            est_cost_delta: (impact > 0.0).then_some(impact),
        });
    }
}

/// Plan ran in parallel and crossed the cost-threshold-for-parallelism
/// boundary. On busy OLTP boxes this often masks a missing-index problem
/// or an over-eager scan. Flag plans whose total subtree cost exceeds
/// the typical CTFP (5 by default) when `<QueryPlan>` declares parallelism.
fn rule_parallelism_cost_warning(stmt: Node, stmt_text: Option<&String>, out: &mut Vec<Finding>) {
    let Some(plan) = stmt.descendants().find(|n| n.has_tag_name("QueryPlan")) else {
        return;
    };
    let degree: i32 = plan
        .attribute("DegreeOfParallelism")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    if degree <= 1 {
        return;
    }
    // The cost on the root RelOp is the whole-plan cost.
    let total = plan
        .children()
        .find(|n| n.has_tag_name("RelOp"))
        .map_or(0.0, estimate_cost);
    if total < 5.0 {
        return;
    }
    out.push(Finding {
        rule: "parallelism-cost-warning",
        severity: Severity::Info,
        statement_text: stmt_text.cloned(),
        target: None,
        message: format!(
            "plan ran with DOP={degree} at estimated cost {total:.1}; on busy OLTP servers this often hides a missing index or unselective scan. Re-check seek-ability before tuning MAXDOP."
        ),
        est_cost_delta: None,
    });
}

/// Columns referenced by a join/predicate with no statistics.
/// `<ColumnsWithNoStatistics>` lists them.
fn rule_join_without_statistics(stmt: Node, stmt_text: Option<&String>, out: &mut Vec<Finding>) {
    for grp in stmt
        .descendants()
        .filter(|n| n.has_tag_name("ColumnsWithNoStatistics"))
    {
        let cols: Vec<String> = grp
            .descendants()
            .filter(|n| n.has_tag_name("ColumnReference"))
            .map(|c| {
                let s = c.attribute("Schema").unwrap_or("").trim_matches(['[', ']']);
                let t = c.attribute("Table").unwrap_or("").trim_matches(['[', ']']);
                let n = c.attribute("Column").unwrap_or("").trim_matches(['[', ']']);
                format!("{s}.{t}.{n}")
            })
            .collect();
        if cols.is_empty() {
            continue;
        }
        let target = cols.first().cloned();
        out.push(Finding {
            rule: "join-without-statistics",
            severity: Severity::Warning,
            statement_text: stmt_text.cloned(),
            target,
            message: format!(
                "no column statistics on: {}. The optimizer is guessing cardinality; run `UPDATE STATISTICS` or enable AUTO_CREATE_STATISTICS.",
                cols.join(", ")
            ),
            est_cost_delta: None,
        });
    }
}

/// Memory grant is large compared to the rows being processed.
/// SQL Server reports `SerialDesiredMemory` (KB). Flag grants above
/// ~512 MB on plans returning <1 M rows — typically a sort/hash that
/// overestimated the row size.
fn rule_excessive_memory_grant(stmt: Node, stmt_text: Option<&String>, out: &mut Vec<Finding>) {
    let Some(plan) = stmt.descendants().find(|n| n.has_tag_name("QueryPlan")) else {
        return;
    };
    let granted_kb: f64 = plan
        .attribute("SerialDesiredMemory")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0.0);
    if granted_kb < 512.0 * 1024.0 {
        return;
    }
    let root_rows = plan
        .children()
        .find(|n| n.has_tag_name("RelOp"))
        .map_or(0.0, estimate_rows);
    if root_rows >= 1_000_000.0 {
        return;
    }
    out.push(Finding {
        rule: "excessive-memory-grant",
        severity: Severity::Warning,
        statement_text: stmt_text.cloned(),
        target: None,
        message: format!(
            "plan requested {:.0} MB of memory to return ~{:.0} rows; check for sort/hash on wide rows or a bad cardinality estimate.",
            granted_kb / 1024.0,
            root_rows
        ),
        est_cost_delta: None,
    });
}

// ---------------------------------------------------------------------------
// Per-RelOp rules
// ---------------------------------------------------------------------------

/// Table / clustered-index scan with a residual predicate (no seek).
fn rule_scan_with_residual(relop: Node, stmt_text: Option<&String>, out: &mut Vec<Finding>) {
    let physical = relop.attribute("PhysicalOp").unwrap_or("");
    let is_scan = matches!(
        physical,
        "Table Scan" | "Clustered Index Scan" | "Index Scan"
    );
    if !is_scan {
        return;
    }
    let has_seek = relop
        .descendants()
        .any(|n| n.has_tag_name("SeekPredicates") || n.has_tag_name("SeekPredicate"));
    if has_seek {
        return;
    }
    let card = table_cardinality(relop);
    // Scans on tiny tables are fine.
    if card != 0.0 && card < 100.0 {
        return;
    }
    let est_rows = estimate_rows(relop);
    let target = scan_object(relop);
    let (rule, msg) = if physical == "Table Scan" {
        (
            "table-scan-no-index",
            format!(
                "heap table is being scanned; add a clustered index (or a covering nonclustered index on the WHERE/JOIN predicate). Cardinality ~{card:.0}."
            ),
        )
    } else {
        (
            "clustered-index-scan-with-residual",
            format!(
                "{physical} returns ~{est_rows:.0} of ~{card:.0} rows but the predicate could not be pushed into a seek; add an index whose leading column matches the WHERE/JOIN predicate."
            ),
        )
    };
    out.push(Finding {
        rule,
        severity: Severity::Warning,
        statement_text: stmt_text.cloned(),
        target,
        message: msg,
        est_cost_delta: Some(estimate_cost(relop)),
    });
}

/// Key Lookup / RID Lookup on a non-covering index.
fn rule_non_covering_lookup(relop: Node, stmt_text: Option<&String>, out: &mut Vec<Finding>) {
    let physical = relop.attribute("PhysicalOp").unwrap_or("");
    let lookup_via_clustered_seek = physical == "Clustered Index Seek"
        && relop
            .descendants()
            .any(|n| n.has_tag_name("IndexScan") && n.attribute("Lookup") == Some("1"));
    let kind = if physical == "Key Lookup" || lookup_via_clustered_seek {
        Some("key-lookup-on-non-covering-index")
    } else if physical == "RID Lookup" {
        Some("rid-lookup")
    } else {
        None
    };
    let Some(rule) = kind else { return };

    let outer_index = relop
        .ancestors()
        .skip(1)
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
                        .and_then(|o| {
                            o.attribute("Index")
                                .map(|s| s.trim_matches(['[', ']']).to_string())
                        })
                })
        });
    let target = scan_object(relop);
    let msg = match (rule, outer_index) {
        ("rid-lookup", _) => {
            "RID Lookup against a heap; add a clustered index, or INCLUDE the SELECTed columns in the nonclustered index used by the outer seek.".to_string()
        }
        (_, Some(idx)) => format!(
            "non-covering index {idx} forces a {physical}; INCLUDE the SELECTed columns in {idx} to eliminate the lookup."
        ),
        _ => format!("{physical} present; the chosen index does not cover the SELECT list."),
    };
    out.push(Finding {
        rule,
        severity: Severity::Warning,
        statement_text: stmt_text.cloned(),
        target,
        message: msg,
        est_cost_delta: Some(estimate_cost(relop)),
    });
}

/// Implicit conversion on a column inside a predicate.
fn rule_implicit_conversion(relop: Node, stmt_text: Option<&String>, out: &mut Vec<Finding>) {
    for conv in relop.descendants().filter(|n| n.has_tag_name("Convert")) {
        if conv.attribute("Implicit") != Some("1") {
            continue;
        }
        let in_predicate = conv.ancestors().any(|a| {
            a.has_tag_name("Predicate")
                || a.has_tag_name("SeekPredicate")
                || a.has_tag_name("SeekPredicateNew")
        });
        if !in_predicate {
            continue;
        }
        let Some(col_ref) = conv
            .descendants()
            .find(|n| n.has_tag_name("ColumnReference"))
        else {
            continue;
        };
        let col = {
            let s = col_ref
                .attribute("Schema")
                .unwrap_or("")
                .trim_matches(['[', ']']);
            let t = col_ref
                .attribute("Table")
                .unwrap_or("")
                .trim_matches(['[', ']']);
            let n = col_ref
                .attribute("Column")
                .unwrap_or("")
                .trim_matches(['[', ']']);
            format!("{s}.{t}.{n}")
        };
        let from = conv.attribute("DataType").unwrap_or("?");
        out.push(Finding {
            rule: "implicit-conversion-on-column",
            severity: Severity::Warning,
            statement_text: stmt_text.cloned(),
            target: Some(col),
            message: format!(
                "implicit conversion to {from} on a column reference inside a predicate; the optimizer cannot seek the underlying index. Match the parameter type to the column type (e.g. `N'foo'` for `nvarchar`)."
            ),
            est_cost_delta: None,
        });
    }
}

/// Hash join / hash aggregate that spilled to tempdb.
fn rule_hash_spill(relop: Node, stmt_text: Option<&String>, out: &mut Vec<Finding>) {
    let physical = relop.attribute("PhysicalOp").unwrap_or("");
    if !matches!(physical, "Hash Match") {
        return;
    }
    let spilled = relop.descendants().any(|n| {
        n.has_tag_name("SpillToTempDb")
            || (n.has_tag_name("Warnings")
                && n.descendants().any(|c| c.has_tag_name("SpillToTempDb")))
    });
    if !spilled {
        return;
    }
    let target = scan_object(relop);
    out.push(Finding {
        rule: "hash-spill-to-tempdb",
        severity: Severity::Warning,
        statement_text: stmt_text.cloned(),
        target,
        message: "hash build side did not fit in the granted memory and spilled to tempdb; expect tempdb pressure. Look at row-size estimates (often a fat NVARCHAR(MAX)) and consider a merge-join-friendly index ordering.".to_string(),
        est_cost_delta: Some(estimate_cost(relop)),
    });
}

/// Sort that spilled / warned. `<Warnings><SortWarning>` is the
/// modern form; older plans use `SpillToTempDb` directly on the Sort.
fn rule_sort_warning(relop: Node, stmt_text: Option<&String>, out: &mut Vec<Finding>) {
    let physical = relop.attribute("PhysicalOp").unwrap_or("");
    if physical != "Sort" {
        return;
    }
    let warned = relop.descendants().any(|n| {
        n.has_tag_name("SortWarning")
            || n.has_tag_name("SpillToTempDb")
            || (n.has_tag_name("Warnings")
                && n.descendants().any(|c| c.has_tag_name("SortWarning")))
    });
    if !warned {
        return;
    }
    out.push(Finding {
        rule: "sort-warning",
        severity: Severity::Warning,
        statement_text: stmt_text.cloned(),
        target: None,
        message: "Sort operator spilled to tempdb; the memory grant was too small. Try a covering index whose leading columns match ORDER BY, or rewrite to avoid the sort.".to_string(),
        est_cost_delta: Some(estimate_cost(relop)),
    });
}

/// Parameter sniffing tell — an `Index Seek` / `Clustered Index Seek`
/// whose `EstimateRows` is dramatically lower than the consumer's
/// expectation, leaving downstream operators sized for a different
/// shape of data. We approximate this in cardinality-only plans by
/// flagging a Seek whose `EstimateRows` is < 1% of `TableCardinality`
/// and whose parent operator is a memory-consumer (Sort / Hash Match).
fn rule_parameter_sniffing(relop: Node, stmt_text: Option<&String>, out: &mut Vec<Finding>) {
    let physical = relop.attribute("PhysicalOp").unwrap_or("");
    if !matches!(physical, "Index Seek" | "Clustered Index Seek") {
        return;
    }
    let est = estimate_rows(relop);
    let card = table_cardinality(relop);
    if card < 10_000.0 || est <= 0.0 {
        return;
    }
    if est / card > 0.01 {
        return;
    }
    let parent_is_memory_consumer = relop
        .ancestors()
        .skip(1)
        .find(|n| n.has_tag_name("RelOp"))
        .is_some_and(|p| {
            matches!(
                p.attribute("PhysicalOp").unwrap_or(""),
                "Sort" | "Hash Match"
            )
        });
    if !parent_is_memory_consumer {
        return;
    }
    let target = scan_object(relop);
    out.push(Finding {
        rule: "parameter-sniffing-tell",
        severity: Severity::Info,
        statement_text: stmt_text.cloned(),
        target,
        message: format!(
            "seek estimates ~{est:.0} of ~{card:.0} rows feeding a Sort/Hash; this shape is sensitive to parameter sniffing. Consider OPTION (RECOMPILE) for outlier params, or OPTIMIZE FOR UNKNOWN."
        ),
        est_cost_delta: None,
    });
}

/// Nested loops with a large outer side — the inner side will be
/// executed for each outer row, which is fine for ~hundreds of rows
/// and terrible for tens of thousands.
fn rule_nested_loops_large_outer(relop: Node, stmt_text: Option<&String>, out: &mut Vec<Finding>) {
    if relop.attribute("PhysicalOp").unwrap_or("") != "Nested Loops" {
        return;
    }
    // Outer = first child RelOp.
    let Some(outer) = relop.children().find(|n| n.has_tag_name("RelOp")) else {
        return;
    };
    let outer_rows = estimate_rows(outer);
    if outer_rows < 10_000.0 {
        return;
    }
    out.push(Finding {
        rule: "nested-loops-on-large-outer",
        severity: Severity::Warning,
        statement_text: stmt_text.cloned(),
        target: None,
        message: format!(
            "Nested Loops join driven by ~{outer_rows:.0} outer rows; consider an index that enables a Merge/Hash join, or rewrite the join order. Each outer row reruns the inner subtree."
        ),
        est_cost_delta: Some(estimate_cost(relop)),
    });
}

/// Eager spool / Lazy spool warning. Eager Spool materializes the
/// inner subtree into tempdb; usually a missing index symptom.
fn rule_eager_spool(relop: Node, stmt_text: Option<&String>, out: &mut Vec<Finding>) {
    let physical = relop.attribute("PhysicalOp").unwrap_or("");
    if !matches!(
        physical,
        "Eager Spool" | "Table Spool" | "Index Spool" | "Row Count Spool"
    ) {
        return;
    }
    // Anchor the rule to Eager Spools — Lazy variants are usually OK.
    let logical = relop.attribute("LogicalOp").unwrap_or("");
    let eager = physical.starts_with("Eager") || logical == "Eager Spool";
    if !eager {
        return;
    }
    out.push(Finding {
        rule: "eager-spool-warning",
        severity: Severity::Warning,
        statement_text: stmt_text.cloned(),
        target: None,
        message: "Eager Spool materializes the inner subtree to tempdb before consuming it (Halloween-protection or a missing supporting index). Add a covering index on the affected predicate to remove the spool.".to_string(),
        est_cost_delta: Some(estimate_cost(relop)),
    });
}

/// Scalar UDF invoked from inside a `<Predicate>`. Pre-SQL 2019
/// these are not inlined and force row-by-row evaluation (TF 8666).
fn rule_udf_in_predicate(relop: Node, stmt_text: Option<&String>, out: &mut Vec<Finding>) {
    for pred in relop.descendants().filter(|n| {
        n.has_tag_name("Predicate")
            || n.has_tag_name("SeekPredicate")
            || n.has_tag_name("SeekPredicateNew")
    }) {
        for udf in pred
            .descendants()
            .filter(|n| n.has_tag_name("UserDefinedFunction"))
        {
            let name = udf
                .attribute("FunctionName")
                .map(|s| s.trim_matches(['[', ']']).to_string());
            out.push(Finding {
                rule: "udf-in-predicate",
                severity: Severity::Warning,
                statement_text: stmt_text.cloned(),
                target: name.clone(),
                message: format!(
                    "scalar UDF {} invoked inside a predicate forces row-by-row evaluation and blocks index seeks. Inline the expression, switch to an inline TVF, or upgrade to SQL Server 2019+ with scalar UDF inlining.",
                    name.as_deref().unwrap_or("(unknown)")
                ),
                est_cost_delta: None,
            });
        }
    }
}
