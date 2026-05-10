//! # mssql-showplan
//!
//! Parse SQL Server `SHOWPLAN_XML` and `STATISTICS XML` output into a typed
//! plan tree.
//!
//! SQL Server produces XML execution plans via `SET SHOWPLAN_XML ON` (estimated)
//! or `SET STATISTICS XML ON` (actual). This crate parses that XML into
//! [`ShowPlan`] / [`PlanNode`] structs you can inspect programmatically.
//!
//! # Examples
//!
//! ```rust
//! use mssql_showplan::parse;
//!
//! let xml = r#"<?xml version="1.0"?>
//! <ShowPlanXML xmlns="http://schemas.microsoft.com/sqlserver/2004/07/showplan">
//!   <BatchSequence><Batch><Statements>
//!     <StmtSimple StatementText="SELECT 1" QueryHash="0xABC">
//!       <QueryPlan CachedPlanSize="16" CompileTime="1"
//!                  CompileCPU="1" CompileMemory="64">
//!         <RelOp NodeId="1" PhysicalOp="Constant Scan"
//!                LogicalOp="Constant Scan" EstimateRows="1"
//!                EstimateCPU="0.0001" EstimateIO="0"
//!                EstimatedTotalSubtreeCost="0.0001"
//!                Parallel="false" />
//!       </QueryPlan>
//!     </StmtSimple>
//!   </Statements></Batch></BatchSequence>
//! </ShowPlanXML>"#;
//!
//! let plan = parse(xml).unwrap();
//! assert_eq!(plan.root.physical_op, "Constant Scan");
//! ```

use std::collections::HashMap;

use quick_xml::events::Event;
use quick_xml::Reader;

// Re-export serde_json only when the serde feature is active.
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

mod error;
pub use error::Error;

/// Result type alias for this crate.
pub type Result<T> = std::result::Result<T, Error>;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A single node in a SQL Server query execution plan.
///
/// Each `<RelOp>` element in the XML maps to one `PlanNode`. Nodes form a
/// tree via [`children`](PlanNode::children).
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct PlanNode {
    /// The `NodeId` attribute from the XML.
    pub node_id: u32,
    /// Physical operator name (e.g. `"Clustered Index Scan"`).
    pub physical_op: String,
    /// Logical operator name (e.g. `"Clustered Index Scan"`).
    pub logical_op: String,
    /// Estimated number of rows produced by this operator.
    pub estimated_rows: Option<f64>,
    /// Estimated CPU cost for this operator.
    pub estimated_cpu: Option<f64>,
    /// Estimated I/O cost for this operator.
    pub estimated_io: Option<f64>,
    /// Estimated total subtree cost (cumulative).
    pub estimated_total_subtree_cost: Option<f64>,
    /// Whether this operator runs in parallel mode.
    pub parallel: bool,
    /// Actual rows returned (from `STATISTICS XML` / actual plans).
    pub actual_rows: Option<f64>,
    /// Actual number of executions (from actual plans).
    pub actual_executions: Option<u64>,
    /// Remaining XML attributes not captured by named fields.
    pub extra: HashMap<String, serde_json::Value>,
    /// Child operators in the plan tree.
    pub children: Vec<PlanNode>,
}

/// Metadata about the query plan extracted from `<StmtSimple>` and
/// `<QueryPlan>` elements.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ShowPlan {
    /// The root operator tree.
    pub root: PlanNode,
    /// The original SQL statement text (`StatementText` attribute).
    pub statement_text: Option<String>,
    /// Query hash for plan-cache identification.
    pub query_hash: Option<String>,
    /// Cached plan size in KB.
    pub cached_plan_size: Option<u32>,
    /// Compilation wall-clock time in ms.
    pub compile_time: Option<u32>,
    /// Compilation CPU time in ms.
    pub compile_cpu: Option<u32>,
    /// Compilation memory grant in KB.
    pub compile_memory: Option<u32>,
}

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

/// Parse SQL Server `SHOWPLAN_XML` or `STATISTICS XML` into a [`ShowPlan`].
///
/// Handles both estimated plans (`SET SHOWPLAN_XML ON`) and actual plans
/// (`SET STATISTICS XML ON`). For actual plans the `actual_rows` and
/// `actual_executions` fields on [`PlanNode`] are populated from the
/// `<RunTimeInformation>` / `<RunTimeCountersPerThread>` elements.
///
/// # Errors
///
/// Returns [`Error::Xml`] on malformed XML, or [`Error::EmptyPlan`] when no
/// `<RelOp>` elements are found.
///
/// # Examples
///
/// ```rust
/// # use mssql_showplan::parse;
/// let xml = include_str!("data/estimated_plan.xml");
/// let plan = parse(xml).unwrap();
/// assert!(!plan.root.physical_op.is_empty());
/// ```
pub fn parse(xml: &str) -> Result<ShowPlan> {
    let mut reader = Reader::from_str(xml);
    let mut buf = Vec::new();

    let mut node_stack: Vec<PlanNode> = Vec::new();
    let mut statement_text: Option<String> = None;
    let mut query_hash: Option<String> = None;
    let mut cached_plan_size: Option<u32> = None;
    let mut compile_time: Option<u32> = None;
    let mut compile_cpu: Option<u32> = None;
    let mut compile_memory: Option<u32> = None;
    let mut node_id_counter: u32 = 0;

    // Sentinel root so children always have a parent to attach to.
    node_stack.push(PlanNode {
        node_id: 0,
        physical_op: String::new(),
        logical_op: String::new(),
        estimated_rows: None,
        estimated_cpu: None,
        estimated_io: None,
        estimated_total_subtree_cost: None,
        parallel: false,
        actual_rows: None,
        actual_executions: None,
        extra: HashMap::new(),
        children: Vec::new(),
    });

    loop {
        let event = reader.read_event_into(&mut buf);
        match event {
            Ok(Event::Start(ref e)) => {
                let local_name = e.local_name();
                let tag = local_name.as_ref();
                match tag {
                    b"StmtSimple" | b"StmtCond" | b"StmtCursor" | b"StmtUseDb" => {
                        parse_statement_attrs(e, &mut statement_text, &mut query_hash);
                    }
                    b"QueryPlan" => {
                        parse_query_plan_attrs(
                            e,
                            &mut cached_plan_size,
                            &mut compile_time,
                            &mut compile_cpu,
                            &mut compile_memory,
                        );
                    }
                    b"RelOp" => {
                        node_id_counter += 1;
                        let node = parse_relop_attrs(e, node_id_counter);
                        node_stack.push(node);
                    }
                    b"RunTimeCountersPerThread" => {
                        // Actual-plan runtime stats. Attach to current RelOp.
                        if let Some(current) = node_stack.last_mut() {
                            parse_runtime_counters(e, current);
                        }
                    }
                    _ => {}
                }
            }
            Ok(Event::Empty(ref e)) => {
                let local_name = e.local_name();
                let tag = local_name.as_ref();
                match tag {
                    b"StmtSimple" | b"StmtCond" | b"StmtCursor" | b"StmtUseDb" => {
                        parse_statement_attrs(e, &mut statement_text, &mut query_hash);
                    }
                    b"QueryPlan" => {
                        parse_query_plan_attrs(
                            e,
                            &mut cached_plan_size,
                            &mut compile_time,
                            &mut compile_cpu,
                            &mut compile_memory,
                        );
                    }
                    b"RelOp" => {
                        // Self-closing RelOp — leaf node, push and immediately pop.
                        node_id_counter += 1;
                        let node = parse_relop_attrs(e, node_id_counter);
                        if let Some(parent) = node_stack.last_mut() {
                            parent.children.push(node);
                        }
                    }
                    b"RunTimeCountersPerThread" => {
                        if let Some(current) = node_stack.last_mut() {
                            parse_runtime_counters(e, current);
                        }
                    }
                    _ => {}
                }
            }
            Ok(Event::End(ref e)) => {
                let local_name = e.local_name();
                if local_name.as_ref() == b"RelOp" {
                    if let Some(finished) = node_stack.pop() {
                        if let Some(parent) = node_stack.last_mut() {
                            parent.children.push(finished);
                        } else {
                            // Shouldn't happen with sentinel, but be safe.
                            node_stack.push(finished);
                        }
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(Error::Xml(e)),
            _ => {}
        }
        buf.clear();
    }

    // The sentinel root should contain the top-level RelOp(s) as children.
    let sentinel = node_stack
        .pop()
        .ok_or(Error::EmptyPlan)?;

    let root = if sentinel.children.len() == 1 {
        sentinel.children.into_iter().next().unwrap()
    } else if sentinel.children.is_empty() {
        return Err(Error::EmptyPlan);
    } else {
        // Multiple top-level operators — wrap in a synthetic root.
        PlanNode {
            node_id: 0,
            physical_op: "Root".to_string(),
            logical_op: "Root".to_string(),
            estimated_rows: None,
            estimated_cpu: None,
            estimated_io: None,
            estimated_total_subtree_cost: None,
            parallel: false,
            actual_rows: None,
            actual_executions: None,
            extra: HashMap::new(),
            children: sentinel.children,
        }
    };

    Ok(ShowPlan {
        root,
        statement_text,
        query_hash,
        cached_plan_size,
        compile_time,
        compile_cpu,
        compile_memory,
    })
}

// ---------------------------------------------------------------------------
// Attribute parsers
// ---------------------------------------------------------------------------

fn attr_str(e: &quick_xml::events::BytesStart<'_>, name: &[u8]) -> Option<String> {
    e.attributes()
        .flatten()
        .find(|a| a.key.as_ref() == name)
        .map(|a| String::from_utf8_lossy(&a.value).into_owned())
}

fn attr_parse<T: std::str::FromStr>(e: &quick_xml::events::BytesStart<'_>, name: &[u8]) -> Option<T> {
    attr_str(e, name).and_then(|s| s.parse().ok())
}

fn parse_statement_attrs(
    e: &quick_xml::events::BytesStart<'_>,
    statement_text: &mut Option<String>,
    query_hash: &mut Option<String>,
) {
    if statement_text.is_none() {
        *statement_text = attr_str(e, b"StatementText");
    }
    if query_hash.is_none() {
        *query_hash = attr_str(e, b"QueryHash");
    }
}

fn parse_query_plan_attrs(
    e: &quick_xml::events::BytesStart<'_>,
    cached_plan_size: &mut Option<u32>,
    compile_time: &mut Option<u32>,
    compile_cpu: &mut Option<u32>,
    compile_memory: &mut Option<u32>,
) {
    if cached_plan_size.is_none() {
        *cached_plan_size = attr_parse(e, b"CachedPlanSize");
    }
    if compile_time.is_none() {
        *compile_time = attr_parse(e, b"CompileTime");
    }
    if compile_cpu.is_none() {
        *compile_cpu = attr_parse(e, b"CompileCPU");
    }
    if compile_memory.is_none() {
        *compile_memory = attr_parse(e, b"CompileMemory");
    }
}

fn parse_relop_attrs(e: &quick_xml::events::BytesStart<'_>, fallback_id: u32) -> PlanNode {
    let mut node = PlanNode {
        node_id: fallback_id,
        physical_op: String::new(),
        logical_op: String::new(),
        estimated_rows: None,
        estimated_cpu: None,
        estimated_io: None,
        estimated_total_subtree_cost: None,
        parallel: false,
        actual_rows: None,
        actual_executions: None,
        extra: HashMap::new(),
        children: Vec::new(),
    };

    for attr in e.attributes().flatten() {
        let key = String::from_utf8_lossy(attr.key.as_ref());
        let val = String::from_utf8_lossy(&attr.value);
        match key.as_ref() {
            "NodeId" => node.node_id = val.parse().unwrap_or(fallback_id),
            "PhysicalOp" => node.physical_op = val.into_owned(),
            "LogicalOp" => node.logical_op = val.into_owned(),
            "EstimateRows" => node.estimated_rows = val.parse().ok(),
            "EstimateCPU" => node.estimated_cpu = val.parse().ok(),
            "EstimateIO" => node.estimated_io = val.parse().ok(),
            "EstimatedTotalSubtreeCost" => {
                node.estimated_total_subtree_cost = val.parse().ok();
            }
            "Parallel" => node.parallel = val.as_ref() == "true" || val.as_ref() == "1",
            _ => {
                // Stash remaining attributes in `extra`.
                let json_val = if let Ok(n) = val.parse::<f64>() {
                    serde_json::Value::Number(
                        serde_json::Number::from_f64(n)
                            .unwrap_or_else(|| serde_json::Number::from(0)),
                    )
                } else if val.as_ref() == "true" || val.as_ref() == "false" {
                    serde_json::Value::Bool(val.as_ref() == "true")
                } else {
                    serde_json::Value::String(val.into_owned())
                };
                node.extra.insert(key.into_owned(), json_val);
            }
        }
    }

    node
}

fn parse_runtime_counters(e: &quick_xml::events::BytesStart<'_>, node: &mut PlanNode) {
    // Thread 0 carries the totals. For simplicity, sum across all threads.
    let rows: Option<f64> = attr_parse(e, b"ActualRows");
    let execs: Option<u64> = attr_parse(e, b"ActualExecutions");

    if let Some(r) = rows {
        node.actual_rows = Some(node.actual_rows.unwrap_or(0.0) + r);
    }
    if let Some(ex) = execs {
        node.actual_executions = Some(node.actual_executions.unwrap_or(0) + ex);
    }
}

#[cfg(test)]
mod tests;
