//! # sqldev-showplan
//!
//! Parse SQL Server `SHOWPLAN_XML` and `STATISTICS XML` output into a typed
//! plan tree.
//!
//! SQL Server produces XML execution plans via `SET SHOWPLAN_XML ON` (estimated)
//! or `SET STATISTICS XML ON` (actual). This crate parses that XML into
//! [`ShowPlan`] / [`PlanNode`] structs you can inspect programmatically.
//!
//! The types model the most actionable parts of the official `showplanxml.xsd`
//! schema (SQL Server 2019+), including warnings, missing indexes, memory
//! grants, wait statistics, runtime counters per thread, output lists, and
//! parameter lists, plus the immediate `<Predicate>` / `<Object>` references
//! that anti-pattern analyzers typically need.
//!
//! # Single vs multi-statement batches
//!
//! A single SHOWPLAN XML document may describe multiple statements
//! (`<StmtSimple>`, `<StmtCond>`, …). Use [`parse_all`] to retrieve every
//! statement's plan as a [`Vec`], or [`parse`] when you know the input is a
//! single statement (it errors with [`Error::MultipleStatements`] otherwise).
//!
//! # Encoding
//!
//! SQL Server returns SHOWPLAN XML as UTF‑16 over the TDS protocol. If you
//! read the raw bytes (e.g. through `tiberius`), decode them to a UTF‑8
//! [`String`] before passing to [`parse`] / [`parse_all`].
//!
//! # Examples
//!
//! ```rust
//! use sqldev_showplan::parse;
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

use std::collections::BTreeMap;

use quick_xml::events::{BytesStart, Event};
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

/// A reference to a base object (table / index / view) accessed by an
/// operator, captured from the immediate `<Object>` element under a physical
/// operator (e.g. `<IndexScan><Object .../></IndexScan>`).
#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ObjectRef {
    /// Server name.
    pub server: Option<String>,
    /// Database name.
    pub database: Option<String>,
    /// Schema name.
    pub schema: Option<String>,
    /// Table or view name.
    pub table: Option<String>,
    /// Index name (when the operator targets a specific index).
    pub index: Option<String>,
    /// Index kind (e.g. `Clustered`, `NonClustered`).
    pub index_kind: Option<String>,
    /// Storage type (e.g. `RowStore`, `ColumnStore`).
    pub storage: Option<String>,
    /// Table alias.
    pub alias: Option<String>,
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
#[derive(Debug, Clone, Default)]
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
#[derive(Debug, Clone, Default)]
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
#[derive(Debug, Clone, Default)]
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
#[derive(Debug, Clone, Default)]
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
/// Per-thread values are summed; `actual_elapsed_ms` takes the max across
/// threads (wall clock).
#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct RuntimeStats {
    /// Total actual rows produced.
    pub actual_rows: u64,
    /// Total actual executions.
    pub actual_executions: u64,
    /// Total end-of-scans counter.
    pub actual_end_of_scans: Option<u64>,
    /// Wall-clock elapsed time in ms (max across threads).
    pub actual_elapsed_ms: Option<u64>,
    /// CPU time in ms (summed across threads).
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
#[derive(Debug, Clone, Default)]
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
    /// Predicate string captured from the immediate `<Predicate>` element of
    /// this operator, when present (e.g. residual predicate on a scan).
    pub predicate: Option<String>,
    /// Base object (table / index / view) accessed by this operator, when
    /// applicable (Scan, Seek, Insert, etc.).
    pub object: Option<ObjectRef>,
    /// Columns in the output list of this operator.
    pub output_list: Vec<ColumnReference>,
    /// Warnings associated with this operator.
    pub warnings: Vec<Warning>,
    /// Runtime execution statistics (from actual plans).
    pub runtime_stats: Option<RuntimeStats>,
    /// Remaining XML attributes not captured by named fields.
    /// Stored as raw strings to keep the dependency surface minimal; insertion
    /// order is preserved by using a [`BTreeMap`].
    pub extra: BTreeMap<String, String>,
    /// Child operators in the plan tree.
    pub children: Vec<PlanNode>,
}

impl PlanNode {
    /// Look up an attribute by name, checking modeled fields first and falling
    /// back to [`PlanNode::extra`].
    ///
    /// Returns the canonical SQL Server XML attribute name spelling
    /// (e.g. `"EstimateRows"`, not `"estimated_rows"`).
    #[must_use]
    pub fn attr(&self, name: &str) -> Option<String> {
        match name {
            "NodeId" => Some(self.node_id.to_string()),
            "PhysicalOp" => Some(self.physical_op.clone()),
            "LogicalOp" => Some(self.logical_op.clone()),
            "EstimateRows" => self.estimated_rows.map(|v| v.to_string()),
            "EstimateCPU" => self.estimated_cpu.map(|v| v.to_string()),
            "EstimateIO" => self.estimated_io.map(|v| v.to_string()),
            "EstimatedTotalSubtreeCost" => {
                self.estimated_total_subtree_cost.map(|v| v.to_string())
            }
            "Parallel" => Some(self.parallel.to_string()),
            "AvgRowSize" => self.avg_row_size.map(|v| v.to_string()),
            "EstimateRebinds" => self.estimated_rebinds.map(|v| v.to_string()),
            "EstimateRewinds" => self.estimated_rewinds.map(|v| v.to_string()),
            "EstimatedExecutionMode" => self.estimated_execution_mode.clone(),
            "Partitioned" => self.partitioned.map(|v| v.to_string()),
            "EstimatedRowsRead" => self.estimated_rows_read.map(|v| v.to_string()),
            other => self.extra.get(other).cloned(),
        }
    }
}

/// Metadata about a query plan extracted from `<StmtSimple>` and
/// `<QueryPlan>` elements.
#[derive(Debug, Clone, Default)]
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
// Parser entry points
// ---------------------------------------------------------------------------

/// Parse SQL Server `SHOWPLAN_XML` or `STATISTICS XML` containing exactly one
/// statement.
///
/// # Errors
///
/// * [`Error::Xml`] on malformed XML.
/// * [`Error::EmptyPlan`] when no `<RelOp>` elements are found.
/// * [`Error::MultipleStatements`] when the XML contains more than one
///   statement plan; use [`parse_all`] instead.
///
/// # Examples
///
/// ```rust
/// # use sqldev_showplan::parse;
/// let xml = include_str!("../tests/data/estimated_plan.xml");
/// let plan = parse(xml).unwrap();
/// assert!(!plan.root.physical_op.is_empty());
/// ```
pub fn parse(xml: &str) -> Result<ShowPlan> {
    let mut all = parse_all(xml)?;
    match all.len() {
        0 => Err(Error::EmptyPlan),
        1 => Ok(all.remove(0)),
        n => Err(Error::MultipleStatements(n)),
    }
}

/// A batch of [`ShowPlan`]s — one per statement in the original XML document.
///
/// Returned by `<&str as TryInto<ShowPlanBatch>>::try_into` and
/// `str::parse::<ShowPlanBatch>()`. Use this when the input may contain more
/// than one statement (i.e. anything other than a single `SELECT`/`INSERT`/
/// `UPDATE`/`DELETE`).
#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ShowPlanBatch(pub Vec<ShowPlan>);

impl ShowPlanBatch {
    /// Number of statement plans in the batch.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// `true` if the batch contains zero plans.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Borrow the plans as a slice.
    #[must_use]
    pub fn as_slice(&self) -> &[ShowPlan] {
        &self.0
    }

    /// Consume the batch, returning the underlying `Vec<ShowPlan>`.
    #[must_use]
    pub fn into_inner(self) -> Vec<ShowPlan> {
        self.0
    }
}

impl IntoIterator for ShowPlanBatch {
    type Item = ShowPlan;
    type IntoIter = std::vec::IntoIter<ShowPlan>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl<'a> IntoIterator for &'a ShowPlanBatch {
    type Item = &'a ShowPlan;
    type IntoIter = std::slice::Iter<'a, ShowPlan>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

impl std::str::FromStr for ShowPlan {
    type Err = Error;

    /// Parse a single-statement SHOWPLAN XML document.
    ///
    /// Returns [`Error::MultipleStatements`] if the document contains more
    /// than one statement plan — use [`ShowPlanBatch`] for batches.
    fn from_str(s: &str) -> Result<Self> {
        parse(s)
    }
}

impl TryFrom<&str> for ShowPlan {
    type Error = Error;
    fn try_from(s: &str) -> Result<Self> {
        s.parse()
    }
}

impl TryFrom<&String> for ShowPlan {
    type Error = Error;
    fn try_from(s: &String) -> Result<Self> {
        s.as_str().parse()
    }
}

impl TryFrom<String> for ShowPlan {
    type Error = Error;
    fn try_from(s: String) -> Result<Self> {
        s.as_str().parse()
    }
}

impl std::str::FromStr for ShowPlanBatch {
    type Err = Error;

    /// Parse a SHOWPLAN XML document containing one or more statement plans.
    fn from_str(s: &str) -> Result<Self> {
        parse_all(s).map(ShowPlanBatch)
    }
}

impl TryFrom<&str> for ShowPlanBatch {
    type Error = Error;
    fn try_from(s: &str) -> Result<Self> {
        s.parse()
    }
}

impl TryFrom<&String> for ShowPlanBatch {
    type Error = Error;
    fn try_from(s: &String) -> Result<Self> {
        s.as_str().parse()
    }
}

impl TryFrom<String> for ShowPlanBatch {
    type Error = Error;
    fn try_from(s: String) -> Result<Self> {
        s.as_str().parse()
    }
}

impl From<Vec<ShowPlan>> for ShowPlanBatch {
    fn from(v: Vec<ShowPlan>) -> Self {
        Self(v)
    }
}

impl From<ShowPlanBatch> for Vec<ShowPlan> {
    fn from(b: ShowPlanBatch) -> Self {
        b.0
    }
}

/// Parse SQL Server `SHOWPLAN_XML` or `STATISTICS XML` and return one
/// [`ShowPlan`] per statement (`<StmtSimple>`, `<StmtCond>`, `<StmtCursor>`,
/// `<StmtUseDb>`) that contains at least one `<RelOp>`.
///
/// Statements with no `<RelOp>` (e.g. `SET NOCOUNT ON`) are silently skipped.
///
/// # Errors
///
/// * [`Error::Xml`] on malformed XML.
/// * [`Error::EmptyPlan`] when no statement contains any `<RelOp>` elements.
pub fn parse_all(xml: &str) -> Result<Vec<ShowPlan>> {
    let mut reader = Reader::from_str(xml);
    let mut buf = Vec::new();
    let mut results: Vec<ShowPlan> = Vec::new();
    let mut state: Option<StatementState> = None;

    loop {
        let event = reader.read_event_into(&mut buf);
        match event {
            Ok(Event::Start(ref e)) => {
                let local = e.local_name();
                let tag = local.as_ref();
                if is_stmt_tag(tag) {
                    if let Some(prev) = state.take() {
                        if let Some(plan) = prev.finalize() {
                            results.push(plan);
                        }
                    }
                    let mut s = StatementState::new();
                    s.read_stmt_attrs(e);
                    state = Some(s);
                } else if let Some(s) = state.as_mut() {
                    s.handle_start(tag, e);
                }
            }
            Ok(Event::Empty(ref e)) => {
                let local = e.local_name();
                let tag = local.as_ref();
                if is_stmt_tag(tag) {
                    if let Some(prev) = state.take() {
                        if let Some(plan) = prev.finalize() {
                            results.push(plan);
                        }
                    }
                    let mut s = StatementState::new();
                    s.read_stmt_attrs(e);
                    if let Some(plan) = s.finalize() {
                        results.push(plan);
                    }
                } else if let Some(s) = state.as_mut() {
                    s.handle_empty(tag, e);
                }
            }
            Ok(Event::End(ref e)) => {
                let local = e.local_name();
                let tag = local.as_ref();
                if is_stmt_tag(tag) {
                    if let Some(prev) = state.take() {
                        if let Some(plan) = prev.finalize() {
                            results.push(plan);
                        }
                    }
                } else if let Some(s) = state.as_mut() {
                    s.handle_end(tag);
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(Error::Xml(e)),
            _ => {}
        }
        buf.clear();
    }

    if let Some(prev) = state.take() {
        if let Some(plan) = prev.finalize() {
            results.push(plan);
        }
    }

    if results.is_empty() {
        return Err(Error::EmptyPlan);
    }
    Ok(results)
}

fn is_stmt_tag(tag: &[u8]) -> bool {
    matches!(
        tag,
        b"StmtSimple" | b"StmtCond" | b"StmtCursor" | b"StmtUseDb"
    )
}

// ---------------------------------------------------------------------------
// Per-statement state machine
// ---------------------------------------------------------------------------

struct StatementState {
    statement_text: Option<String>,
    query_hash: Option<String>,
    query_plan_hash: Option<String>,
    statement_type: Option<String>,
    optimization_level: Option<String>,
    early_abort_reason: Option<String>,
    cardinality_estimation_model: Option<String>,
    cached_plan_size: Option<u32>,
    compile_time: Option<u32>,
    compile_cpu: Option<u32>,
    compile_memory: Option<u32>,
    degree_of_parallelism: Option<u32>,
    non_parallel_plan_reason: Option<String>,
    missing_indexes: Vec<MissingIndex>,
    memory_grant: Option<MemoryGrant>,
    wait_stats: Vec<WaitStats>,
    parameters: Vec<ColumnReference>,

    node_stack: Vec<PlanNode>,
    node_id_counter: u32,

    in_output_list: bool,
    in_warnings: bool,
    in_columns_with_no_statistics: bool,
    in_missing_indexes: bool,
    in_wait_stats: bool,
    in_parameter_list: bool,
    in_predicate: bool,

    in_missing_index_element: bool,
    current_missing_index_impact: f64,
    current_missing_index_database: String,
    current_missing_index_schema: String,
    current_missing_index_table: String,
    current_missing_index_equality: Vec<String>,
    current_missing_index_inequality: Vec<String>,
    current_missing_index_include: Vec<String>,
    current_column_group_usage: Option<String>,
}

impl StatementState {
    fn new() -> Self {
        let mut s = Self {
            statement_text: None,
            query_hash: None,
            query_plan_hash: None,
            statement_type: None,
            optimization_level: None,
            early_abort_reason: None,
            cardinality_estimation_model: None,
            cached_plan_size: None,
            compile_time: None,
            compile_cpu: None,
            compile_memory: None,
            degree_of_parallelism: None,
            non_parallel_plan_reason: None,
            missing_indexes: Vec::new(),
            memory_grant: None,
            wait_stats: Vec::new(),
            parameters: Vec::new(),
            node_stack: Vec::new(),
            node_id_counter: 0,
            in_output_list: false,
            in_warnings: false,
            in_columns_with_no_statistics: false,
            in_missing_indexes: false,
            in_wait_stats: false,
            in_parameter_list: false,
            in_predicate: false,
            in_missing_index_element: false,
            current_missing_index_impact: 0.0,
            current_missing_index_database: String::new(),
            current_missing_index_schema: String::new(),
            current_missing_index_table: String::new(),
            current_missing_index_equality: Vec::new(),
            current_missing_index_inequality: Vec::new(),
            current_missing_index_include: Vec::new(),
            current_column_group_usage: None,
        };
        // Sentinel root so children always have a parent to attach to. The
        // sentinel uses u32::MAX as its id to avoid clashing with real node ids.
        s.node_stack.push(make_empty_node(u32::MAX));
        s
    }

    fn read_stmt_attrs(&mut self, e: &BytesStart<'_>) {
        if self.statement_text.is_none() {
            self.statement_text = attr_str(e, b"StatementText");
        }
        if self.query_hash.is_none() {
            self.query_hash = attr_str(e, b"QueryHash");
        }
        if self.query_plan_hash.is_none() {
            self.query_plan_hash = attr_str(e, b"QueryPlanHash");
        }
        if self.statement_type.is_none() {
            self.statement_type = attr_str(e, b"StatementType");
        }
        if self.optimization_level.is_none() {
            self.optimization_level = attr_str(e, b"StatementOptmLevel");
        }
        if self.early_abort_reason.is_none() {
            self.early_abort_reason = attr_str(e, b"StatementOptmEarlyAbortReason");
        }
        if self.cardinality_estimation_model.is_none() {
            self.cardinality_estimation_model =
                attr_str(e, b"CardinalityEstimationModelVersion");
        }
    }

    fn read_query_plan_attrs(&mut self, e: &BytesStart<'_>) {
        if self.cached_plan_size.is_none() {
            self.cached_plan_size = attr_parse(e, b"CachedPlanSize");
        }
        if self.compile_time.is_none() {
            self.compile_time = attr_parse(e, b"CompileTime");
        }
        if self.compile_cpu.is_none() {
            self.compile_cpu = attr_parse(e, b"CompileCPU");
        }
        if self.compile_memory.is_none() {
            self.compile_memory = attr_parse(e, b"CompileMemory");
        }
        if self.degree_of_parallelism.is_none() {
            self.degree_of_parallelism = attr_parse(e, b"DegreeOfParallelism");
        }
        if self.non_parallel_plan_reason.is_none() {
            self.non_parallel_plan_reason = attr_str(e, b"NonParallelPlanReason");
        }
    }

    fn push_warning_with_attrs(&mut self, e: &BytesStart<'_>) {
        let mut warning = Warning::default();
        if let Some(val) = attr_str(e, b"NoJoinPredicate") {
            warning.no_join_predicate = parse_bool(&val);
        }
        if let Some(current) = self.node_stack.last_mut() {
            current.warnings.push(warning);
        }
    }

    fn handle_start(&mut self, tag: &[u8], e: &BytesStart<'_>) {
        match tag {
            b"QueryPlan" => self.read_query_plan_attrs(e),
            b"RelOp" => {
                self.node_id_counter += 1;
                let node = parse_relop_attrs(e, self.node_id_counter);
                self.node_stack.push(node);
            }
            b"OutputList" => self.in_output_list = true,
            b"Warnings" => {
                self.in_warnings = true;
                self.push_warning_with_attrs(e);
            }
            b"ColumnsWithNoStatistics" if self.in_warnings => {
                self.in_columns_with_no_statistics = true;
            }
            b"SpillToTempDb" if self.in_warnings => {
                let detail = parse_spill_detail(e);
                if let Some(current) = self.node_stack.last_mut() {
                    if let Some(w) = current.warnings.last_mut() {
                        w.spill_to_temp_db.push(detail);
                    }
                }
            }
            b"SpillOccurred" if self.in_warnings => {
                if let Some(current) = self.node_stack.last_mut() {
                    if let Some(w) = current.warnings.last_mut() {
                        w.spill_occurred = true;
                    }
                }
            }
            b"MemoryGrantWarning" if self.in_warnings => {
                let mgw = parse_memory_grant_warning(e);
                if let Some(current) = self.node_stack.last_mut() {
                    if let Some(w) = current.warnings.last_mut() {
                        w.memory_grant_warning = Some(mgw);
                    }
                }
            }
            b"MissingIndexes" => self.in_missing_indexes = true,
            b"MissingIndexGroup" if self.in_missing_indexes => {
                self.current_missing_index_impact = attr_parse(e, b"Impact").unwrap_or(0.0);
            }
            b"MissingIndex" if self.in_missing_indexes => {
                self.current_missing_index_database =
                    attr_str(e, b"Database").unwrap_or_default();
                self.current_missing_index_schema =
                    attr_str(e, b"Schema").unwrap_or_default();
                self.current_missing_index_table = attr_str(e, b"Table").unwrap_or_default();
                self.current_missing_index_equality.clear();
                self.current_missing_index_inequality.clear();
                self.current_missing_index_include.clear();
                self.in_missing_index_element = true;
            }
            b"ColumnGroup" if self.in_missing_index_element => {
                self.current_column_group_usage = attr_str(e, b"Usage");
            }
            b"MemoryGrantInfo" => {
                self.memory_grant = Some(parse_memory_grant(e));
            }
            b"WaitStats" => self.in_wait_stats = true,
            b"ParameterList" => self.in_parameter_list = true,
            b"Predicate" => self.in_predicate = true,
            b"Object" if !self.in_missing_index_element => {
                if let Some(current) = self.node_stack.last_mut() {
                    if current.object.is_none() {
                        current.object = Some(parse_object_ref(e));
                    }
                }
            }
            b"RunTimeCountersPerThread" => {
                if let Some(current) = self.node_stack.last_mut() {
                    parse_runtime_counters(e, current);
                }
            }
            b"ScalarOperator" if self.in_predicate => {
                if let Some(current) = self.node_stack.last_mut() {
                    if current.predicate.is_none() {
                        if let Some(s) = attr_str(e, b"ScalarString") {
                            current.predicate = Some(s);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn handle_empty(&mut self, tag: &[u8], e: &BytesStart<'_>) {
        match tag {
            b"QueryPlan" => self.read_query_plan_attrs(e),
            b"RelOp" => {
                self.node_id_counter += 1;
                let node = parse_relop_attrs(e, self.node_id_counter);
                if let Some(parent) = self.node_stack.last_mut() {
                    parent.children.push(node);
                }
            }
            b"ColumnReference" if self.in_output_list => {
                if let Some(current) = self.node_stack.last_mut() {
                    current.output_list.push(parse_column_reference(e));
                }
            }
            b"ColumnReference" if self.in_columns_with_no_statistics => {
                if let Some(current) = self.node_stack.last_mut() {
                    if let Some(w) = current.warnings.last_mut() {
                        w.columns_with_no_statistics.push(parse_column_reference(e));
                    }
                }
            }
            b"ColumnReference" if self.in_parameter_list => {
                self.parameters.push(parse_column_reference(e));
            }
            b"Column" if self.in_missing_index_element => {
                if let Some(name) = attr_str(e, b"Name") {
                    match self.current_column_group_usage.as_deref() {
                        Some("EQUALITY") => self.current_missing_index_equality.push(name),
                        Some("INEQUALITY") => {
                            self.current_missing_index_inequality.push(name);
                        }
                        Some("INCLUDE") => self.current_missing_index_include.push(name),
                        _ => {}
                    }
                }
            }
            b"SpillToTempDb" if self.in_warnings => {
                let detail = parse_spill_detail(e);
                if let Some(current) = self.node_stack.last_mut() {
                    if let Some(w) = current.warnings.last_mut() {
                        w.spill_to_temp_db.push(detail);
                    }
                }
            }
            b"SpillOccurred" if self.in_warnings => {
                if let Some(current) = self.node_stack.last_mut() {
                    if let Some(w) = current.warnings.last_mut() {
                        w.spill_occurred = true;
                    }
                }
            }
            b"MemoryGrantWarning" if self.in_warnings => {
                let mgw = parse_memory_grant_warning(e);
                if let Some(current) = self.node_stack.last_mut() {
                    if let Some(w) = current.warnings.last_mut() {
                        w.memory_grant_warning = Some(mgw);
                    }
                }
            }
            b"Warnings" => self.push_warning_with_attrs(e),
            b"MemoryGrantInfo" => self.memory_grant = Some(parse_memory_grant(e)),
            b"Wait" if self.in_wait_stats => {
                if let Some(ws) = parse_wait_stat(e) {
                    self.wait_stats.push(ws);
                }
            }
            b"RunTimeCountersPerThread" => {
                if let Some(current) = self.node_stack.last_mut() {
                    parse_runtime_counters(e, current);
                }
            }
            b"Object" if !self.in_missing_index_element => {
                if let Some(current) = self.node_stack.last_mut() {
                    if current.object.is_none() {
                        current.object = Some(parse_object_ref(e));
                    }
                }
            }
            b"ScalarOperator" if self.in_predicate => {
                if let Some(current) = self.node_stack.last_mut() {
                    if current.predicate.is_none() {
                        if let Some(s) = attr_str(e, b"ScalarString") {
                            current.predicate = Some(s);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn handle_end(&mut self, tag: &[u8]) {
        match tag {
            b"RelOp" => {
                if let Some(finished) = self.node_stack.pop() {
                    if let Some(parent) = self.node_stack.last_mut() {
                        parent.children.push(finished);
                    } else {
                        // Should never happen because of the sentinel; restore.
                        self.node_stack.push(finished);
                    }
                }
            }
            b"OutputList" => self.in_output_list = false,
            b"Warnings" => {
                self.in_warnings = false;
                self.in_columns_with_no_statistics = false;
            }
            b"ColumnsWithNoStatistics" => self.in_columns_with_no_statistics = false,
            b"MissingIndexes" => self.in_missing_indexes = false,
            b"MissingIndex" if self.in_missing_indexes => {
                self.missing_indexes.push(MissingIndex {
                    impact: self.current_missing_index_impact,
                    database: std::mem::take(&mut self.current_missing_index_database),
                    schema: std::mem::take(&mut self.current_missing_index_schema),
                    table: std::mem::take(&mut self.current_missing_index_table),
                    equality_columns: std::mem::take(&mut self.current_missing_index_equality),
                    inequality_columns: std::mem::take(
                        &mut self.current_missing_index_inequality,
                    ),
                    include_columns: std::mem::take(&mut self.current_missing_index_include),
                });
                self.in_missing_index_element = false;
            }
            b"ColumnGroup" => self.current_column_group_usage = None,
            b"WaitStats" => self.in_wait_stats = false,
            b"ParameterList" => self.in_parameter_list = false,
            b"Predicate" => self.in_predicate = false,
            _ => {}
        }
    }

    fn finalize(mut self) -> Option<ShowPlan> {
        let sentinel = self.node_stack.pop()?;
        let sentinel_warnings = sentinel.warnings;

        let mut root = match sentinel.children.len() {
            0 => return None,
            1 => sentinel.children.into_iter().next().unwrap(),
            _ => PlanNode {
                node_id: u32::MAX,
                physical_op: "Root".to_string(),
                logical_op: "Root".to_string(),
                children: sentinel.children,
                ..PlanNode::default()
            },
        };

        if !sentinel_warnings.is_empty() {
            let mut merged = sentinel_warnings;
            merged.extend(root.warnings);
            root.warnings = merged;
        }

        Some(ShowPlan {
            root,
            statement_text: self.statement_text,
            query_hash: self.query_hash,
            query_plan_hash: self.query_plan_hash,
            statement_type: self.statement_type,
            optimization_level: self.optimization_level,
            early_abort_reason: self.early_abort_reason,
            cardinality_estimation_model: self.cardinality_estimation_model,
            cached_plan_size: self.cached_plan_size,
            compile_time: self.compile_time,
            compile_cpu: self.compile_cpu,
            compile_memory: self.compile_memory,
            degree_of_parallelism: self.degree_of_parallelism,
            non_parallel_plan_reason: self.non_parallel_plan_reason,
            missing_indexes: std::mem::take(&mut self.missing_indexes),
            memory_grant: self.memory_grant.take(),
            wait_stats: std::mem::take(&mut self.wait_stats),
            parameters: std::mem::take(&mut self.parameters),
        })
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn make_empty_node(node_id: u32) -> PlanNode {
    PlanNode {
        node_id,
        ..PlanNode::default()
    }
}

fn parse_bool(s: &str) -> bool {
    matches!(s, "true" | "1" | "True" | "TRUE")
}

fn attr_str(e: &BytesStart<'_>, name: &[u8]) -> Option<String> {
    e.attributes()
        .flatten()
        .find(|a| a.key.as_ref() == name)
        .map(|a| String::from_utf8_lossy(&a.value).into_owned())
}

fn attr_parse<T: std::str::FromStr>(e: &BytesStart<'_>, name: &[u8]) -> Option<T> {
    attr_str(e, name).and_then(|s| s.parse().ok())
}

fn parse_relop_attrs(e: &BytesStart<'_>, fallback_id: u32) -> PlanNode {
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
            "Parallel" => node.parallel = parse_bool(val.as_ref()),
            "AvgRowSize" => node.avg_row_size = val.parse().ok(),
            "EstimateRebinds" => node.estimated_rebinds = val.parse().ok(),
            "EstimateRewinds" => node.estimated_rewinds = val.parse().ok(),
            "EstimatedExecutionMode" => {
                node.estimated_execution_mode = Some(val.into_owned());
            }
            "Partitioned" => node.partitioned = Some(parse_bool(val.as_ref())),
            "EstimatedRowsRead" => node.estimated_rows_read = val.parse().ok(),
            _ => {
                node.extra.insert(key.into_owned(), val.into_owned());
            }
        }
    }

    node
}

fn parse_column_reference(e: &BytesStart<'_>) -> ColumnReference {
    ColumnReference {
        server: attr_str(e, b"Server"),
        database: attr_str(e, b"Database"),
        schema: attr_str(e, b"Schema"),
        table: attr_str(e, b"Table"),
        alias: attr_str(e, b"Alias"),
        column: attr_str(e, b"Column").unwrap_or_default(),
        computed_column: attr_str(e, b"ComputedColumn").map(|v| parse_bool(&v)),
        parameter_data_type: attr_str(e, b"ParameterDataType"),
        parameter_compiled_value: attr_str(e, b"ParameterCompiledValue"),
        parameter_runtime_value: attr_str(e, b"ParameterRuntimeValue"),
    }
}

fn parse_object_ref(e: &BytesStart<'_>) -> ObjectRef {
    ObjectRef {
        server: attr_str(e, b"Server"),
        database: attr_str(e, b"Database"),
        schema: attr_str(e, b"Schema"),
        table: attr_str(e, b"Table"),
        index: attr_str(e, b"Index"),
        index_kind: attr_str(e, b"IndexKind"),
        storage: attr_str(e, b"Storage"),
        alias: attr_str(e, b"Alias"),
    }
}

fn parse_spill_detail(e: &BytesStart<'_>) -> SpillDetail {
    SpillDetail {
        spill_level: attr_parse(e, b"SpillLevel"),
        spilled_thread_count: attr_parse(e, b"SpilledThreadCount"),
    }
}

fn parse_memory_grant_warning(e: &BytesStart<'_>) -> MemoryGrantWarning {
    MemoryGrantWarning {
        kind: attr_str(e, b"GrantWarningKind").unwrap_or_default(),
        requested: attr_parse(e, b"RequestedMemory").unwrap_or(0),
        granted: attr_parse(e, b"GrantedMemory").unwrap_or(0),
        max_used: attr_parse(e, b"MaxUsedMemory").unwrap_or(0),
    }
}

/// Parse a `<MemoryGrantInfo>` element. Required attributes default to 0 if
/// absent (a malformed plan won't silently swallow the whole struct).
fn parse_memory_grant(e: &BytesStart<'_>) -> MemoryGrant {
    MemoryGrant {
        serial_required_memory: attr_parse(e, b"SerialRequiredMemory").unwrap_or(0),
        serial_desired_memory: attr_parse(e, b"SerialDesiredMemory").unwrap_or(0),
        required_memory: attr_parse(e, b"RequiredMemory"),
        desired_memory: attr_parse(e, b"DesiredMemory"),
        requested_memory: attr_parse(e, b"RequestedMemory"),
        granted_memory: attr_parse(e, b"GrantedMemory"),
        max_used_memory: attr_parse(e, b"MaxUsedMemory"),
        grant_wait_time: attr_parse(e, b"GrantWaitTime"),
    }
}

fn parse_wait_stat(e: &BytesStart<'_>) -> Option<WaitStats> {
    Some(WaitStats {
        wait_type: attr_str(e, b"WaitType")?,
        wait_time_ms: attr_parse(e, b"WaitTimeMs")?,
        wait_count: attr_parse(e, b"WaitCount")?,
    })
}

fn parse_runtime_counters(e: &BytesStart<'_>, node: &mut PlanNode) {
    let stats = node.runtime_stats.get_or_insert_with(RuntimeStats::default);

    if let Some(r) = attr_parse::<u64>(e, b"ActualRows") {
        stats.actual_rows += r;
    }
    if let Some(ex) = attr_parse::<u64>(e, b"ActualExecutions") {
        stats.actual_executions += ex;
    }
    if let Some(eos) = attr_parse::<u64>(e, b"ActualEndOfScans") {
        stats.actual_end_of_scans = Some(stats.actual_end_of_scans.unwrap_or(0) + eos);
    }
    if let Some(elapsed) = attr_parse::<u64>(e, b"ActualElapsedms") {
        stats.actual_elapsed_ms = Some(stats.actual_elapsed_ms.unwrap_or(0).max(elapsed));
    }
    if let Some(cpu) = attr_parse::<u64>(e, b"ActualCPUms") {
        stats.actual_cpu_ms = Some(stats.actual_cpu_ms.unwrap_or(0) + cpu);
    }
    if let Some(s) = attr_parse::<u64>(e, b"ActualScans") {
        stats.actual_scans = Some(stats.actual_scans.unwrap_or(0) + s);
    }
    if let Some(lr) = attr_parse::<u64>(e, b"ActualLogicalReads") {
        stats.actual_logical_reads = Some(stats.actual_logical_reads.unwrap_or(0) + lr);
    }
    if let Some(pr) = attr_parse::<u64>(e, b"ActualPhysicalReads") {
        stats.actual_physical_reads = Some(stats.actual_physical_reads.unwrap_or(0) + pr);
    }
    if let Some(ra) = attr_parse::<u64>(e, b"ActualReadAheads") {
        stats.actual_read_aheads = Some(stats.actual_read_aheads.unwrap_or(0) + ra);
    }
    if let Some(ll) = attr_parse::<u64>(e, b"ActualLobLogicalReads") {
        stats.actual_lob_logical_reads =
            Some(stats.actual_lob_logical_reads.unwrap_or(0) + ll);
    }
    if let Some(lp) = attr_parse::<u64>(e, b"ActualLobPhysicalReads") {
        stats.actual_lob_physical_reads =
            Some(stats.actual_lob_physical_reads.unwrap_or(0) + lp);
    }
}

#[cfg(test)]
mod tests;
