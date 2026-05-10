//! # mssql-showplan
//!
//! Parse SQL Server `SHOWPLAN_XML` and `STATISTICS XML` output into a typed
//! plan tree.
//!
//! SQL Server produces XML execution plans via `SET SHOWPLAN_XML ON` (estimated)
//! or `SET STATISTICS XML ON` (actual). This crate parses that XML into
//! [`ShowPlan`] / [`PlanNode`] structs you can inspect programmatically.
//!
//! The types in this crate model the most actionable parts of the official
//! `showplanxml.xsd` schema (SQL Server 2019), including warnings, missing
//! indexes, memory grants, wait statistics, runtime counters, output lists,
//! and parameter lists.
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
//!                AvgRowSize="11"
//!                EstimateRebinds="0" EstimateRewinds="0"
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

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

mod error;
pub use error::Error;

/// Result type alias for this crate.
pub type Result<T> = std::result::Result<T, Error>;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A reference to a column in the execution plan, corresponding to
/// `ColumnReferenceType` in the XSD schema.
///
/// Used in output lists, parameter lists, warnings, and other plan elements.
#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ColumnReference {
    /// Server name (e.g. `[MyServer]`).
    pub server: Option<String>,
    /// Database name (e.g. `[TestDB]`).
    pub database: Option<String>,
    /// Schema name (e.g. `[dbo]`).
    pub schema: Option<String>,
    /// Table name (e.g. `[Orders]`).
    pub table: Option<String>,
    /// Alias for the table reference.
    pub alias: Option<String>,
    /// Column name (required in the XSD).
    pub column: String,
    /// Whether this is a computed column.
    pub computed_column: Option<bool>,
    /// Data type for parameterized queries.
    pub parameter_data_type: Option<String>,
    /// Compiled value for parameterized queries.
    pub parameter_compiled_value: Option<String>,
    /// Runtime value for parameterized queries.
    pub parameter_runtime_value: Option<String>,
}

/// Details about a tempdb spill, from `SpillToTempDbType` in the XSD.
#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct SpillDetail {
    /// Spill recursion level.
    pub spill_level: Option<u64>,
    /// Number of threads that spilled.
    pub spilled_thread_count: Option<u64>,
}

/// Warning about excessive/insufficient memory grants, from
/// `MemoryGrantWarningInfo` in the XSD.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct MemoryGrantWarning {
    /// Warning kind (e.g. `"Excessive Grant"`, `"Used More Than Granted"`).
    pub kind: String,
    /// Requested memory in KB.
    pub requested: u64,
    /// Granted memory in KB.
    pub granted: u64,
    /// Maximum used memory in KB.
    pub max_used: u64,
}

/// Warnings associated with a query plan or relational operator, from
/// `WarningsType` in the XSD.
///
/// Warnings can appear on both `<QueryPlan>` and individual `<RelOp>` elements.
#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Warning {
    /// Whether the optimizer detected a missing join predicate.
    pub no_join_predicate: bool,
    /// Whether a spill occurred (from `<SpillOccurred>` element).
    pub spill_occurred: bool,
    /// Columns that have no statistics.
    pub columns_with_no_statistics: Vec<ColumnReference>,
    /// Details about tempdb spills.
    pub spill_to_temp_db: Vec<SpillDetail>,
    /// Memory grant warning information.
    pub memory_grant_warning: Option<MemoryGrantWarning>,
}

/// A missing index suggestion from the optimizer, from
/// `MissingIndexGroupType` / `MissingIndexType` in the XSD.
///
/// This is one of the most actionable elements in a query plan — developers
/// look for these to identify indexes that could improve query performance.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct MissingIndex {
    /// Estimated improvement impact (0–100).
    pub impact: f64,
    /// Database name.
    pub database: String,
    /// Schema name.
    pub schema: String,
    /// Table name.
    pub table: String,
    /// Columns used in equality predicates.
    pub equality_columns: Vec<String>,
    /// Columns used in inequality predicates.
    pub inequality_columns: Vec<String>,
    /// Columns to include (covering).
    pub include_columns: Vec<String>,
}

/// Memory grant information from `MemoryGrantType` in the XSD, found under
/// `<MemoryGrantInfo>` in the `<QueryPlan>` element.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct MemoryGrant {
    /// Required memory in KB for serial execution.
    pub serial_required_memory: u64,
    /// Desired memory in KB for serial execution.
    pub serial_desired_memory: u64,
    /// Required memory in KB for the chosen parallelism.
    pub required_memory: Option<u64>,
    /// Desired memory in KB for the chosen parallelism.
    pub desired_memory: Option<u64>,
    /// Requested memory from the memory manager in KB.
    pub requested_memory: Option<u64>,
    /// Actually granted memory in KB.
    pub granted_memory: Option<u64>,
    /// Maximum memory used during execution in KB.
    pub max_used_memory: Option<u64>,
    /// Time waited for memory grant in seconds.
    pub grant_wait_time: Option<u64>,
}

/// Wait statistics during query execution, from `WaitStatType` in the XSD.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct WaitStats {
    /// Name of the wait type (e.g. `"CXPACKET"`, `"PAGEIOLATCH_SH"`).
    pub wait_type: String,
    /// Total wait time in milliseconds.
    pub wait_time_ms: u64,
    /// Number of waits.
    pub wait_count: u64,
}

/// Runtime execution statistics aggregated across threads, from
/// `RunTimeCountersPerThread` elements in the XSD.
///
/// Values are summed across all threads in a parallel plan.
#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct RuntimeStats {
    /// Total actual rows produced.
    pub actual_rows: u64,
    /// Total actual executions.
    pub actual_executions: u64,
    /// Elapsed time in milliseconds.
    pub actual_elapsed_ms: Option<u64>,
    /// CPU time in milliseconds.
    pub actual_cpu_ms: Option<u64>,
    /// Number of scans performed.
    pub actual_scans: Option<u64>,
    /// Logical reads performed.
    pub actual_logical_reads: Option<u64>,
    /// Physical reads performed.
    pub actual_physical_reads: Option<u64>,
    /// Read-ahead reads performed.
    pub actual_read_aheads: Option<u64>,
    /// LOB logical reads performed.
    pub actual_lob_logical_reads: Option<u64>,
    /// LOB physical reads performed.
    pub actual_lob_physical_reads: Option<u64>,
}

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
    /// Average row size in bytes (from `AvgRowSize` attribute).
    pub avg_row_size: Option<f64>,
    /// Estimated number of rebinds.
    pub estimated_rebinds: Option<f64>,
    /// Estimated number of rewinds.
    pub estimated_rewinds: Option<f64>,
    /// Estimated execution mode (`"Row"` or `"Batch"`).
    pub estimated_execution_mode: Option<String>,
    /// Whether this is a partitioned operator.
    pub partitioned: Option<bool>,
    /// Estimated number of rows read (may differ from `estimated_rows`).
    pub estimated_rows_read: Option<f64>,
    /// Columns in the output list of this operator.
    pub output_list: Vec<ColumnReference>,
    /// Warnings associated with this operator.
    pub warnings: Vec<Warning>,
    /// Runtime execution statistics (from actual plans).
    pub runtime_stats: Option<RuntimeStats>,
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
    /// Query plan hash.
    pub query_plan_hash: Option<String>,
    /// Statement type (e.g. `"SELECT"`, `"INSERT"`, `"UPDATE"`, `"DELETE"`).
    pub statement_type: Option<String>,
    /// Optimization level (e.g. `"FULL"`, `"TRIVIAL"`).
    pub optimization_level: Option<String>,
    /// Early abort reason if the optimizer stopped early.
    pub early_abort_reason: Option<String>,
    /// Cardinality estimation model version.
    pub cardinality_estimation_model: Option<String>,
    /// Cached plan size in KB.
    pub cached_plan_size: Option<u32>,
    /// Compilation wall-clock time in ms.
    pub compile_time: Option<u32>,
    /// Compilation CPU time in ms.
    pub compile_cpu: Option<u32>,
    /// Compilation memory grant in KB.
    pub compile_memory: Option<u32>,
    /// Degree of parallelism for this query plan.
    pub degree_of_parallelism: Option<u32>,
    /// Reason the optimizer chose a non-parallel plan.
    pub non_parallel_plan_reason: Option<String>,
    /// Missing index suggestions from the optimizer.
    pub missing_indexes: Vec<MissingIndex>,
    /// Memory grant information.
    pub memory_grant: Option<MemoryGrant>,
    /// Wait statistics from actual execution.
    pub wait_stats: Vec<WaitStats>,
    /// Parameter list with compiled and runtime values.
    pub parameters: Vec<ColumnReference>,
}

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

/// Parse SQL Server `SHOWPLAN_XML` or `STATISTICS XML` into a [`ShowPlan`].
///
/// Handles both estimated plans (`SET SHOWPLAN_XML ON`) and actual plans
/// (`SET STATISTICS XML ON`). For actual plans the [`RuntimeStats`] on
/// [`PlanNode`] is populated from `<RunTimeInformation>` /
/// `<RunTimeCountersPerThread>` elements.
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
    let mut query_plan_hash: Option<String> = None;
    let mut statement_type: Option<String> = None;
    let mut optimization_level: Option<String> = None;
    let mut early_abort_reason: Option<String> = None;
    let mut cardinality_estimation_model: Option<String> = None;
    let mut cached_plan_size: Option<u32> = None;
    let mut compile_time: Option<u32> = None;
    let mut compile_cpu: Option<u32> = None;
    let mut compile_memory: Option<u32> = None;
    let mut degree_of_parallelism: Option<u32> = None;
    let mut non_parallel_plan_reason: Option<String> = None;
    let mut missing_indexes: Vec<MissingIndex> = Vec::new();
    let mut memory_grant: Option<MemoryGrant> = None;
    let mut wait_stats: Vec<WaitStats> = Vec::new();
    let mut parameters: Vec<ColumnReference> = Vec::new();
    let mut node_id_counter: u32 = 0;

    // Parser state tracking
    let mut in_output_list = false;
    let mut in_warnings = false;
    let mut in_columns_with_no_statistics = false;
    let mut in_missing_indexes = false;
    let mut in_wait_stats = false;
    let mut in_parameter_list = false;

    // Missing index accumulation state
    let mut current_missing_index_impact: f64 = 0.0;
    let mut current_missing_index_database: String = String::new();
    let mut current_missing_index_schema: String = String::new();
    let mut current_missing_index_table: String = String::new();
    let mut current_missing_index_equality: Vec<String> = Vec::new();
    let mut current_missing_index_inequality: Vec<String> = Vec::new();
    let mut current_missing_index_include: Vec<String> = Vec::new();
    let mut current_column_group_usage: Option<String> = None;
    let mut in_missing_index_element = false;

    // Sentinel root so children always have a parent to attach to.
    node_stack.push(make_empty_node(0));

    loop {
        let event = reader.read_event_into(&mut buf);
        match event {
            Ok(Event::Start(ref e)) => {
                let local_name = e.local_name();
                let tag = local_name.as_ref();
                match tag {
                    b"StmtSimple" | b"StmtCond" | b"StmtCursor" | b"StmtUseDb" => {
                        parse_statement_attrs(
                            e,
                            &mut statement_text,
                            &mut query_hash,
                            &mut query_plan_hash,
                            &mut statement_type,
                            &mut optimization_level,
                            &mut early_abort_reason,
                            &mut cardinality_estimation_model,
                        );
                    }
                    b"QueryPlan" => {
                        parse_query_plan_attrs(
                            e,
                            &mut cached_plan_size,
                            &mut compile_time,
                            &mut compile_cpu,
                            &mut compile_memory,
                            &mut degree_of_parallelism,
                            &mut non_parallel_plan_reason,
                        );
                    }
                    b"RelOp" => {
                        node_id_counter += 1;
                        let node = parse_relop_attrs(e, node_id_counter);
                        node_stack.push(node);
                    }
                    b"OutputList" => {
                        in_output_list = true;
                    }
                    b"Warnings" => {
                        in_warnings = true;
                        // Parse Warnings attributes (NoJoinPredicate, etc.)
                        if let Some(current) = node_stack.last_mut() {
                            let mut warning = Warning::default();
                            if let Some(val) = attr_str(e, b"NoJoinPredicate") {
                                warning.no_join_predicate =
                                    val == "true" || val == "1";
                            }
                            current.warnings.push(warning);
                        }
                    }
                    b"ColumnsWithNoStatistics" if in_warnings => {
                        in_columns_with_no_statistics = true;
                    }
                    b"SpillToTempDb" if in_warnings => {
                        let detail = parse_spill_detail(e);
                        if let Some(current) = node_stack.last_mut() {
                            if let Some(w) = current.warnings.last_mut() {
                                w.spill_to_temp_db.push(detail);
                            }
                        }
                    }
                    b"SpillOccurred" if in_warnings => {
                        if let Some(current) = node_stack.last_mut() {
                            if let Some(w) = current.warnings.last_mut() {
                                w.spill_occurred = true;
                            }
                        }
                    }
                    b"MemoryGrantWarning" if in_warnings => {
                        let mgw = parse_memory_grant_warning(e);
                        if let Some(current) = node_stack.last_mut() {
                            if let Some(w) = current.warnings.last_mut() {
                                w.memory_grant_warning = Some(mgw);
                            }
                        }
                    }
                    b"MissingIndexes" => {
                        in_missing_indexes = true;
                    }
                    b"MissingIndexGroup" if in_missing_indexes => {
                        current_missing_index_impact =
                            attr_parse(e, b"Impact").unwrap_or(0.0);
                    }
                    b"MissingIndex" if in_missing_indexes => {
                        current_missing_index_database =
                            attr_str(e, b"Database").unwrap_or_default();
                        current_missing_index_schema =
                            attr_str(e, b"Schema").unwrap_or_default();
                        current_missing_index_table =
                            attr_str(e, b"Table").unwrap_or_default();
                        current_missing_index_equality.clear();
                        current_missing_index_inequality.clear();
                        current_missing_index_include.clear();
                        in_missing_index_element = true;
                    }
                    b"ColumnGroup" if in_missing_index_element => {
                        current_column_group_usage = attr_str(e, b"Usage");
                    }
                    b"MemoryGrantInfo" => {
                        memory_grant = parse_memory_grant(e);
                    }
                    b"WaitStats" => {
                        in_wait_stats = true;
                    }
                    b"ParameterList" => {
                        in_parameter_list = true;
                    }
                    b"RunTimeCountersPerThread" => {
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
                        parse_statement_attrs(
                            e,
                            &mut statement_text,
                            &mut query_hash,
                            &mut query_plan_hash,
                            &mut statement_type,
                            &mut optimization_level,
                            &mut early_abort_reason,
                            &mut cardinality_estimation_model,
                        );
                    }
                    b"QueryPlan" => {
                        parse_query_plan_attrs(
                            e,
                            &mut cached_plan_size,
                            &mut compile_time,
                            &mut compile_cpu,
                            &mut compile_memory,
                            &mut degree_of_parallelism,
                            &mut non_parallel_plan_reason,
                        );
                    }
                    b"RelOp" => {
                        node_id_counter += 1;
                        let node = parse_relop_attrs(e, node_id_counter);
                        if let Some(parent) = node_stack.last_mut() {
                            parent.children.push(node);
                        }
                    }
                    b"ColumnReference" if in_output_list => {
                        let col_ref = parse_column_reference(e);
                        if let Some(current) = node_stack.last_mut() {
                            current.output_list.push(col_ref);
                        }
                    }
                    b"ColumnReference" if in_columns_with_no_statistics => {
                        let col_ref = parse_column_reference(e);
                        if let Some(current) = node_stack.last_mut() {
                            if let Some(w) = current.warnings.last_mut() {
                                w.columns_with_no_statistics.push(col_ref);
                            }
                        }
                    }
                    b"ColumnReference" if in_parameter_list => {
                        let col_ref = parse_column_reference(e);
                        parameters.push(col_ref);
                    }
                    b"Column" if in_missing_index_element => {
                        if let Some(name) = attr_str(e, b"Name") {
                            match current_column_group_usage.as_deref() {
                                Some("EQUALITY") => {
                                    current_missing_index_equality.push(name);
                                }
                                Some("INEQUALITY") => {
                                    current_missing_index_inequality.push(name);
                                }
                                Some("INCLUDE") => {
                                    current_missing_index_include.push(name);
                                }
                                _ => {}
                            }
                        }
                    }
                    b"SpillToTempDb" if in_warnings => {
                        let detail = parse_spill_detail(e);
                        if let Some(current) = node_stack.last_mut() {
                            if let Some(w) = current.warnings.last_mut() {
                                w.spill_to_temp_db.push(detail);
                            }
                        }
                    }
                    b"SpillOccurred" if in_warnings => {
                        if let Some(current) = node_stack.last_mut() {
                            if let Some(w) = current.warnings.last_mut() {
                                w.spill_occurred = true;
                            }
                        }
                    }
                    b"MemoryGrantWarning" if in_warnings => {
                        let mgw = parse_memory_grant_warning(e);
                        if let Some(current) = node_stack.last_mut() {
                            if let Some(w) = current.warnings.last_mut() {
                                w.memory_grant_warning = Some(mgw);
                            }
                        }
                    }
                    b"Warnings" => {
                        // Self-closing <Warnings NoJoinPredicate="true" /> etc.
                        let mut warning = Warning::default();
                        if let Some(val) = attr_str(e, b"NoJoinPredicate") {
                            warning.no_join_predicate = val == "true" || val == "1";
                        }
                        if let Some(current) = node_stack.last_mut() {
                            current.warnings.push(warning);
                        }
                    }
                    b"MemoryGrantInfo" => {
                        memory_grant = parse_memory_grant(e);
                    }
                    b"Wait" if in_wait_stats => {
                        if let Some(ws) = parse_wait_stat(e) {
                            wait_stats.push(ws);
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
                let tag = local_name.as_ref();
                match tag {
                    b"RelOp" => {
                        if let Some(finished) = node_stack.pop() {
                            if let Some(parent) = node_stack.last_mut() {
                                parent.children.push(finished);
                            } else {
                                node_stack.push(finished);
                            }
                        }
                    }
                    b"OutputList" => {
                        in_output_list = false;
                    }
                    b"Warnings" => {
                        in_warnings = false;
                        in_columns_with_no_statistics = false;
                    }
                    b"ColumnsWithNoStatistics" => {
                        in_columns_with_no_statistics = false;
                    }
                    b"MissingIndexes" => {
                        in_missing_indexes = false;
                    }
                    b"MissingIndex" if in_missing_indexes => {
                        missing_indexes.push(MissingIndex {
                            impact: current_missing_index_impact,
                            database: current_missing_index_database.clone(),
                            schema: current_missing_index_schema.clone(),
                            table: current_missing_index_table.clone(),
                            equality_columns: current_missing_index_equality.clone(),
                            inequality_columns: current_missing_index_inequality.clone(),
                            include_columns: current_missing_index_include.clone(),
                        });
                        in_missing_index_element = false;
                    }
                    b"ColumnGroup" => {
                        current_column_group_usage = None;
                    }
                    b"WaitStats" => {
                        in_wait_stats = false;
                    }
                    b"ParameterList" => {
                        in_parameter_list = false;
                    }
                    _ => {}
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(Error::Xml(e)),
            _ => {}
        }
        buf.clear();
    }

    let sentinel = node_stack.pop().ok_or(Error::EmptyPlan)?;

    // Transfer any warnings that were attached to the sentinel (e.g. QueryPlan-level
    // warnings that appeared before any RelOp was opened).
    let sentinel_warnings = sentinel.warnings;

    let mut root = if sentinel.children.len() == 1 {
        sentinel.children.into_iter().next().unwrap()
    } else if sentinel.children.is_empty() {
        return Err(Error::EmptyPlan);
    } else {
        PlanNode {
            node_id: 0,
            physical_op: "Root".to_string(),
            logical_op: "Root".to_string(),
            estimated_rows: None,
            estimated_cpu: None,
            estimated_io: None,
            estimated_total_subtree_cost: None,
            parallel: false,
            avg_row_size: None,
            estimated_rebinds: None,
            estimated_rewinds: None,
            estimated_execution_mode: None,
            partitioned: None,
            estimated_rows_read: None,
            output_list: Vec::new(),
            warnings: Vec::new(),
            runtime_stats: None,
            extra: HashMap::new(),
            children: sentinel.children,
        }
    };

    // Merge sentinel warnings (QueryPlan-level) into the root node.
    if !sentinel_warnings.is_empty() {
        // Prepend them so they appear before any RelOp-level warnings.
        let mut merged = sentinel_warnings;
        merged.extend(root.warnings);
        root.warnings = merged;
    }

    Ok(ShowPlan {
        root,
        statement_text,
        query_hash,
        query_plan_hash,
        statement_type,
        optimization_level,
        early_abort_reason,
        cardinality_estimation_model,
        cached_plan_size,
        compile_time,
        compile_cpu,
        compile_memory,
        degree_of_parallelism,
        non_parallel_plan_reason,
        missing_indexes,
        memory_grant,
        wait_stats,
        parameters,
    })
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn make_empty_node(node_id: u32) -> PlanNode {
    PlanNode {
        node_id,
        physical_op: String::new(),
        logical_op: String::new(),
        estimated_rows: None,
        estimated_cpu: None,
        estimated_io: None,
        estimated_total_subtree_cost: None,
        parallel: false,
        avg_row_size: None,
        estimated_rebinds: None,
        estimated_rewinds: None,
        estimated_execution_mode: None,
        partitioned: None,
        estimated_rows_read: None,
        output_list: Vec::new(),
        warnings: Vec::new(),
        runtime_stats: None,
        extra: HashMap::new(),
        children: Vec::new(),
    }
}

fn attr_str(e: &quick_xml::events::BytesStart<'_>, name: &[u8]) -> Option<String> {
    e.attributes()
        .flatten()
        .find(|a| a.key.as_ref() == name)
        .map(|a| String::from_utf8_lossy(&a.value).into_owned())
}

fn attr_parse<T: std::str::FromStr>(
    e: &quick_xml::events::BytesStart<'_>,
    name: &[u8],
) -> Option<T> {
    attr_str(e, name).and_then(|s| s.parse().ok())
}

#[allow(clippy::too_many_arguments)]
fn parse_statement_attrs(
    e: &quick_xml::events::BytesStart<'_>,
    statement_text: &mut Option<String>,
    query_hash: &mut Option<String>,
    query_plan_hash: &mut Option<String>,
    statement_type: &mut Option<String>,
    optimization_level: &mut Option<String>,
    early_abort_reason: &mut Option<String>,
    cardinality_estimation_model: &mut Option<String>,
) {
    if statement_text.is_none() {
        *statement_text = attr_str(e, b"StatementText");
    }
    if query_hash.is_none() {
        *query_hash = attr_str(e, b"QueryHash");
    }
    if query_plan_hash.is_none() {
        *query_plan_hash = attr_str(e, b"QueryPlanHash");
    }
    if statement_type.is_none() {
        *statement_type = attr_str(e, b"StatementType");
    }
    if optimization_level.is_none() {
        *optimization_level = attr_str(e, b"StatementOptmLevel");
    }
    if early_abort_reason.is_none() {
        *early_abort_reason = attr_str(e, b"StatementOptmEarlyAbortReason");
    }
    if cardinality_estimation_model.is_none() {
        *cardinality_estimation_model =
            attr_str(e, b"CardinalityEstimationModelVersion");
    }
}

fn parse_query_plan_attrs(
    e: &quick_xml::events::BytesStart<'_>,
    cached_plan_size: &mut Option<u32>,
    compile_time: &mut Option<u32>,
    compile_cpu: &mut Option<u32>,
    compile_memory: &mut Option<u32>,
    degree_of_parallelism: &mut Option<u32>,
    non_parallel_plan_reason: &mut Option<String>,
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
    if degree_of_parallelism.is_none() {
        *degree_of_parallelism = attr_parse(e, b"DegreeOfParallelism");
    }
    if non_parallel_plan_reason.is_none() {
        *non_parallel_plan_reason = attr_str(e, b"NonParallelPlanReason");
    }
}

fn parse_relop_attrs(e: &quick_xml::events::BytesStart<'_>, fallback_id: u32) -> PlanNode {
    let mut node = make_empty_node(fallback_id);

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
            "AvgRowSize" => node.avg_row_size = val.parse().ok(),
            "EstimateRebinds" => node.estimated_rebinds = val.parse().ok(),
            "EstimateRewinds" => node.estimated_rewinds = val.parse().ok(),
            "EstimatedExecutionMode" => {
                node.estimated_execution_mode = Some(val.into_owned());
            }
            "Partitioned" => {
                node.partitioned = Some(val.as_ref() == "true" || val.as_ref() == "1");
            }
            "EstimatedRowsRead" => node.estimated_rows_read = val.parse().ok(),
            _ => {
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

fn parse_column_reference(e: &quick_xml::events::BytesStart<'_>) -> ColumnReference {
    ColumnReference {
        server: attr_str(e, b"Server"),
        database: attr_str(e, b"Database"),
        schema: attr_str(e, b"Schema"),
        table: attr_str(e, b"Table"),
        alias: attr_str(e, b"Alias"),
        column: attr_str(e, b"Column").unwrap_or_default(),
        computed_column: attr_str(e, b"ComputedColumn")
            .map(|v| v == "true" || v == "1"),
        parameter_data_type: attr_str(e, b"ParameterDataType"),
        parameter_compiled_value: attr_str(e, b"ParameterCompiledValue"),
        parameter_runtime_value: attr_str(e, b"ParameterRuntimeValue"),
    }
}

fn parse_spill_detail(e: &quick_xml::events::BytesStart<'_>) -> SpillDetail {
    SpillDetail {
        spill_level: attr_parse(e, b"SpillLevel"),
        spilled_thread_count: attr_parse(e, b"SpilledThreadCount"),
    }
}

fn parse_memory_grant_warning(e: &quick_xml::events::BytesStart<'_>) -> MemoryGrantWarning {
    MemoryGrantWarning {
        kind: attr_str(e, b"GrantWarningKind").unwrap_or_default(),
        requested: attr_parse(e, b"RequestedMemory").unwrap_or(0),
        granted: attr_parse(e, b"GrantedMemory").unwrap_or(0),
        max_used: attr_parse(e, b"MaxUsedMemory").unwrap_or(0),
    }
}

fn parse_memory_grant(e: &quick_xml::events::BytesStart<'_>) -> Option<MemoryGrant> {
    Some(MemoryGrant {
        serial_required_memory: attr_parse(e, b"SerialRequiredMemory")?,
        serial_desired_memory: attr_parse(e, b"SerialDesiredMemory")?,
        required_memory: attr_parse(e, b"RequiredMemory"),
        desired_memory: attr_parse(e, b"DesiredMemory"),
        requested_memory: attr_parse(e, b"RequestedMemory"),
        granted_memory: attr_parse(e, b"GrantedMemory"),
        max_used_memory: attr_parse(e, b"MaxUsedMemory"),
        grant_wait_time: attr_parse(e, b"GrantWaitTime"),
    })
}

fn parse_wait_stat(e: &quick_xml::events::BytesStart<'_>) -> Option<WaitStats> {
    Some(WaitStats {
        wait_type: attr_str(e, b"WaitType")?,
        wait_time_ms: attr_parse(e, b"WaitTimeMs")?,
        wait_count: attr_parse(e, b"WaitCount")?,
    })
}

fn parse_runtime_counters(e: &quick_xml::events::BytesStart<'_>, node: &mut PlanNode) {
    let rows: Option<u64> = attr_parse(e, b"ActualRows");
    let execs: Option<u64> = attr_parse(e, b"ActualExecutions");
    let elapsed: Option<u64> = attr_parse(e, b"ActualElapsedms");
    let cpu: Option<u64> = attr_parse(e, b"ActualCPUms");
    let scans: Option<u64> = attr_parse(e, b"ActualScans");
    let logical_reads: Option<u64> = attr_parse(e, b"ActualLogicalReads");
    let physical_reads: Option<u64> = attr_parse(e, b"ActualPhysicalReads");
    let read_aheads: Option<u64> = attr_parse(e, b"ActualReadAheads");
    let lob_logical: Option<u64> = attr_parse(e, b"ActualLobLogicalReads");
    let lob_physical: Option<u64> = attr_parse(e, b"ActualLobPhysicalReads");

    let stats = node.runtime_stats.get_or_insert_with(RuntimeStats::default);

    if let Some(r) = rows {
        stats.actual_rows += r;
    }
    if let Some(ex) = execs {
        stats.actual_executions += ex;
    }
    // For time-based stats, take the max across threads (elapsed) or sum (CPU)
    if let Some(e) = elapsed {
        stats.actual_elapsed_ms =
            Some(stats.actual_elapsed_ms.unwrap_or(0).max(e));
    }
    if let Some(c) = cpu {
        stats.actual_cpu_ms = Some(stats.actual_cpu_ms.unwrap_or(0) + c);
    }
    if let Some(s) = scans {
        stats.actual_scans = Some(stats.actual_scans.unwrap_or(0) + s);
    }
    if let Some(lr) = logical_reads {
        stats.actual_logical_reads =
            Some(stats.actual_logical_reads.unwrap_or(0) + lr);
    }
    if let Some(pr) = physical_reads {
        stats.actual_physical_reads =
            Some(stats.actual_physical_reads.unwrap_or(0) + pr);
    }
    if let Some(ra) = read_aheads {
        stats.actual_read_aheads =
            Some(stats.actual_read_aheads.unwrap_or(0) + ra);
    }
    if let Some(ll) = lob_logical {
        stats.actual_lob_logical_reads =
            Some(stats.actual_lob_logical_reads.unwrap_or(0) + ll);
    }
    if let Some(lp) = lob_physical {
        stats.actual_lob_physical_reads =
            Some(stats.actual_lob_physical_reads.unwrap_or(0) + lp);
    }
}

#[cfg(test)]
mod tests;
