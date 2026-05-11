use crate::{parse, PlanNode};

/// Helper to count all nodes in the tree.
fn count_nodes(node: &PlanNode) -> usize {
    1 + node.children.iter().map(count_nodes).sum::<usize>()
}

// -----------------------------------------------------------------------
// Estimated plan (multi-node join + sort)
// -----------------------------------------------------------------------

#[test]
fn parse_estimated_plan() {
    let xml = include_str!("../tests/data/estimated_plan.xml");
    let plan = parse(xml).expect("should parse estimated plan");

    // Metadata
    assert!(plan
        .statement_text
        .as_deref()
        .unwrap()
        .contains("SELECT o.OrderID"));
    assert_eq!(plan.query_hash.as_deref(), Some("0xA1B2C3D4E5F60718"));
    assert_eq!(plan.query_plan_hash.as_deref(), Some("0x1234567890ABCDEF"));
    assert_eq!(plan.statement_type.as_deref(), Some("SELECT"));
    assert_eq!(plan.optimization_level.as_deref(), Some("FULL"));
    assert_eq!(
        plan.early_abort_reason.as_deref(),
        Some("GoodEnoughPlanFound")
    );
    assert_eq!(plan.cardinality_estimation_model.as_deref(), Some("150"));
    assert_eq!(plan.cached_plan_size, Some(48));
    assert_eq!(plan.compile_time, Some(12));
    assert_eq!(plan.compile_cpu, Some(8));
    assert_eq!(plan.compile_memory, Some(256));
    assert_eq!(plan.degree_of_parallelism, Some(1));

    // Root operator
    assert_eq!(plan.root.physical_op, "Sort");
    assert_eq!(plan.root.logical_op, "Sort");
    assert_eq!(plan.root.node_id, 0);
    assert!(!plan.root.parallel);

    // Cost estimates on root
    assert!((plan.root.estimated_rows.unwrap() - 500.0).abs() < f64::EPSILON);
    assert!((plan.root.estimated_total_subtree_cost.unwrap() - 1.234).abs() < 1e-6);

    // New fields on root
    assert!((plan.root.avg_row_size.unwrap() - 120.0).abs() < f64::EPSILON);
    assert!((plan.root.estimated_rebinds.unwrap() - 0.0).abs() < f64::EPSILON);
    assert!((plan.root.estimated_rewinds.unwrap() - 0.0).abs() < f64::EPSILON);

    // Tree shape: Sort -> Hash Match -> (Clustered Index Scan, Index Seek)
    assert_eq!(plan.root.children.len(), 1, "Sort has one child");
    let hash = &plan.root.children[0];
    assert_eq!(hash.physical_op, "Hash Match");
    assert_eq!(hash.logical_op, "Inner Join");
    assert_eq!(hash.children.len(), 2, "Hash Match has two children");

    let scan = &hash.children[0];
    assert_eq!(scan.physical_op, "Clustered Index Scan");
    assert!(scan.children.is_empty());

    let seek = &hash.children[1];
    assert_eq!(seek.physical_op, "Index Seek");
    assert!(seek.children.is_empty());

    // Total node count
    assert_eq!(count_nodes(&plan.root), 4);

    // Memory grant
    let mg = plan
        .memory_grant
        .as_ref()
        .expect("should have memory grant");
    assert_eq!(mg.serial_required_memory, 512);
    assert_eq!(mg.serial_desired_memory, 1024);
    assert_eq!(mg.granted_memory, Some(1024));
    assert_eq!(mg.max_used_memory, Some(768));
    assert_eq!(mg.grant_wait_time, Some(0));

    // Parameters
    assert_eq!(plan.parameters.len(), 1);
    assert_eq!(plan.parameters[0].column, "@StartDate");
    assert_eq!(
        plan.parameters[0].parameter_data_type.as_deref(),
        Some("datetime")
    );
    assert_eq!(
        plan.parameters[0].parameter_compiled_value.as_deref(),
        Some("'2024-01-01'")
    );
    assert_eq!(
        plan.parameters[0].parameter_runtime_value.as_deref(),
        Some("'2024-01-01'")
    );
}

// -----------------------------------------------------------------------
// Actual plan (STATISTICS XML with RunTimeCountersPerThread)
// -----------------------------------------------------------------------

#[test]
fn parse_actual_plan() {
    let xml = include_str!("../tests/data/actual_plan.xml");
    let plan = parse(xml).expect("should parse actual plan");

    assert_eq!(plan.query_hash.as_deref(), Some("0xFEDCBA9876543210"));

    // Root is Top
    assert_eq!(plan.root.physical_op, "Top");
    assert_eq!(plan.root.children.len(), 1);

    // Child has runtime stats
    let scan = &plan.root.children[0];
    assert_eq!(scan.physical_op, "Clustered Index Scan");

    let stats = scan
        .runtime_stats
        .as_ref()
        .expect("should have runtime stats");
    assert_eq!(stats.actual_rows, 10);
    assert_eq!(stats.actual_executions, 1);
    assert_eq!(stats.actual_elapsed_ms, Some(5));
    assert_eq!(stats.actual_cpu_ms, Some(3));
    assert_eq!(stats.actual_scans, Some(1));
    assert_eq!(stats.actual_logical_reads, Some(42));
    assert_eq!(stats.actual_physical_reads, Some(5));

    // Output list on scan
    assert!(!scan.output_list.is_empty());
    assert_eq!(scan.output_list[0].column, "OrderID");
    assert_eq!(scan.output_list[0].database.as_deref(), Some("[TestDB]"));
}

// -----------------------------------------------------------------------
// Inline estimated plan (no external file)
// -----------------------------------------------------------------------

#[test]
fn parse_inline_simple() {
    let xml = r#"<?xml version="1.0"?>
<ShowPlanXML xmlns="http://schemas.microsoft.com/sqlserver/2004/07/showplan">
  <BatchSequence><Batch><Statements>
    <StmtSimple StatementText="SELECT 1" QueryHash="0x1">
      <QueryPlan CachedPlanSize="8" CompileTime="0" CompileCPU="0" CompileMemory="32">
        <RelOp NodeId="1" PhysicalOp="Constant Scan" LogicalOp="Constant Scan"
               EstimateRows="1" EstimateCPU="0.0001" EstimateIO="0"
               AvgRowSize="11" EstimateRebinds="0" EstimateRewinds="0"
               EstimatedTotalSubtreeCost="0.0001" Parallel="false" />
      </QueryPlan>
    </StmtSimple>
  </Statements></Batch></BatchSequence>
</ShowPlanXML>"#;

    let plan = parse(xml).unwrap();
    assert_eq!(plan.root.physical_op, "Constant Scan");
    assert_eq!(plan.root.node_id, 1);
    assert_eq!(plan.root.estimated_rows, Some(1.0));
    assert!(!plan.root.parallel);
    assert!(plan.root.children.is_empty());
    assert!(plan.root.runtime_stats.is_none());
}

// -----------------------------------------------------------------------
// Parallel plan
// -----------------------------------------------------------------------

#[test]
fn parse_parallel_flag() {
    let xml = r#"<?xml version="1.0"?>
<ShowPlanXML xmlns="http://schemas.microsoft.com/sqlserver/2004/07/showplan">
  <BatchSequence><Batch><Statements>
    <StmtSimple StatementText="SELECT COUNT(*) FROM BigTable">
      <QueryPlan CachedPlanSize="16" CompileTime="5" CompileCPU="3" CompileMemory="128">
        <RelOp NodeId="0" PhysicalOp="Stream Aggregate" LogicalOp="Aggregate"
               EstimateRows="1" EstimateCPU="0.001" EstimateIO="0"
               AvgRowSize="11" EstimateRebinds="0" EstimateRewinds="0"
               EstimatedTotalSubtreeCost="5.0" Parallel="true">
          <OutputList />
          <RelOp NodeId="1" PhysicalOp="Clustered Index Scan" LogicalOp="Clustered Index Scan"
                 EstimateRows="1000000" EstimateCPU="1.0" EstimateIO="3.5"
                 AvgRowSize="11" EstimateRebinds="0" EstimateRewinds="0"
                 EstimatedTotalSubtreeCost="4.5" Parallel="true">
            <OutputList />
          </RelOp>
        </RelOp>
      </QueryPlan>
    </StmtSimple>
  </Statements></Batch></BatchSequence>
</ShowPlanXML>"#;

    let plan = parse(xml).unwrap();
    assert!(plan.root.parallel);
    assert_eq!(plan.root.children.len(), 1);
    assert!(plan.root.children[0].parallel);
}

// -----------------------------------------------------------------------
// Empty / malformed plans
// -----------------------------------------------------------------------

#[test]
fn empty_plan_returns_error() {
    let xml = r#"<?xml version="1.0"?>
<ShowPlanXML xmlns="http://schemas.microsoft.com/sqlserver/2004/07/showplan">
  <BatchSequence><Batch><Statements>
    <StmtSimple StatementText="SET NOCOUNT ON" />
  </Statements></Batch></BatchSequence>
</ShowPlanXML>"#;

    let err = parse(xml).unwrap_err();
    let msg = format!("{err}");
    assert!(
        msg.contains("empty"),
        "Error should mention empty plan: {msg}"
    );
}

// -----------------------------------------------------------------------
// Extra attributes are captured
// -----------------------------------------------------------------------

#[test]
fn extra_attributes_captured() {
    let xml = r#"<?xml version="1.0"?>
<ShowPlanXML xmlns="http://schemas.microsoft.com/sqlserver/2004/07/showplan">
  <BatchSequence><Batch><Statements>
    <StmtSimple StatementText="SELECT 1">
      <QueryPlan CachedPlanSize="8" CompileTime="0" CompileCPU="0" CompileMemory="32">
        <RelOp NodeId="1" PhysicalOp="Constant Scan" LogicalOp="Constant Scan"
               EstimateRows="1" EstimateCPU="0.0001" EstimateIO="0"
               AvgRowSize="11" EstimateRebinds="0" EstimateRewinds="0"
               EstimatedTotalSubtreeCost="0.0001" Parallel="false"
               TableCardinality="42" />
      </QueryPlan>
    </StmtSimple>
  </Statements></Batch></BatchSequence>
</ShowPlanXML>"#;

    let plan = parse(xml).unwrap();
    assert!(plan.root.extra.contains_key("TableCardinality"));
    assert_eq!(plan.root.extra["TableCardinality"], "42");
}

// -----------------------------------------------------------------------
// serde round-trip (compile-time check that derives are present)
// -----------------------------------------------------------------------

#[cfg(feature = "serde")]
#[test]
fn serde_round_trip() {
    let xml = include_str!("../tests/data/estimated_plan.xml");
    let plan = parse(xml).unwrap();
    let json = serde_json::to_string_pretty(&plan).expect("serialize");
    let deser: crate::ShowPlan = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(deser.root.physical_op, plan.root.physical_op);
    assert_eq!(deser.query_hash, plan.query_hash);
    assert_eq!(deser.statement_type, plan.statement_type);
    assert_eq!(
        deser
            .memory_grant
            .as_ref()
            .map(|m| m.serial_required_memory),
        plan.memory_grant.as_ref().map(|m| m.serial_required_memory)
    );
}

// -----------------------------------------------------------------------
// OutputList parsing
// -----------------------------------------------------------------------

#[test]
fn parse_output_list() {
    let xml = include_str!("../tests/data/estimated_plan.xml");
    let plan = parse(xml).unwrap();

    // Root (Sort) should have 3 output columns
    assert_eq!(plan.root.output_list.len(), 3);
    assert_eq!(plan.root.output_list[0].column, "OrderID");
    assert_eq!(
        plan.root.output_list[0].database.as_deref(),
        Some("[TestDB]")
    );
    assert_eq!(plan.root.output_list[0].schema.as_deref(), Some("[dbo]"));
    assert_eq!(plan.root.output_list[0].table.as_deref(), Some("[Orders]"));

    assert_eq!(plan.root.output_list[1].column, "OrderDate");
    assert_eq!(plan.root.output_list[2].column, "CustomerName");

    // Hash Match child also has output list
    let hash = &plan.root.children[0];
    assert_eq!(hash.output_list.len(), 3);
}

// -----------------------------------------------------------------------
// Missing indexes
// -----------------------------------------------------------------------

#[test]
fn parse_missing_indexes() {
    let xml = include_str!("../tests/data/missing_indexes_plan.xml");
    let plan = parse(xml).unwrap();

    assert_eq!(plan.missing_indexes.len(), 1);
    let mi = &plan.missing_indexes[0];
    assert!((mi.impact - 87.5).abs() < f64::EPSILON);
    assert_eq!(mi.database, "[TestDB]");
    assert_eq!(mi.schema, "[dbo]");
    assert_eq!(mi.table, "[Orders]");
    assert_eq!(mi.equality_columns, vec!["[CustomerName]"]);
    assert_eq!(mi.inequality_columns, vec!["[OrderDate]"]);
    assert_eq!(mi.include_columns, vec!["[OrderID]", "[Status]"]);
}

// -----------------------------------------------------------------------
// Warnings (no join predicate, spill, memory grant warning)
// -----------------------------------------------------------------------

#[test]
fn parse_warnings() {
    let xml = include_str!("../tests/data/warnings_plan.xml");
    let plan = parse(xml).unwrap();

    assert_eq!(plan.degree_of_parallelism, Some(4));

    // Hash Match operator (root) has warnings
    let root = &plan.root;
    assert_eq!(root.physical_op, "Hash Match");

    // The root has 2 warnings: one from <QueryPlan><Warnings> and one from the <RelOp><Warnings>
    // The query-level warning is attached to the sentinel and ends up on root
    // Actually in our parser, QueryPlan-level Warnings is not handled on node_stack
    // since there's no RelOp open yet. Let's check the RelOp-level warning.
    assert!(!root.warnings.is_empty(), "root should have warnings");
    let relop_warning = &root.warnings.last().unwrap();
    assert!(relop_warning.spill_occurred);
    assert_eq!(relop_warning.spill_to_temp_db.len(), 2);
    assert_eq!(relop_warning.spill_to_temp_db[0].spill_level, Some(1));
    assert_eq!(
        relop_warning.spill_to_temp_db[0].spilled_thread_count,
        Some(2)
    );
    assert_eq!(relop_warning.spill_to_temp_db[1].spill_level, Some(2));

    let mgw = relop_warning
        .memory_grant_warning
        .as_ref()
        .expect("should have memory grant warning");
    assert_eq!(mgw.kind, "Excessive Grant");
    assert_eq!(mgw.requested, 65536);
    assert_eq!(mgw.granted, 65536);
    assert_eq!(mgw.max_used, 8192);

    // Child Table Scan has columns with no statistics warning
    let table_scan = &root.children[0];
    assert_eq!(table_scan.physical_op, "Table Scan");
    assert!(!table_scan.warnings.is_empty());
    let child_warning = &table_scan.warnings[0];
    assert_eq!(child_warning.columns_with_no_statistics.len(), 1);
    assert_eq!(child_warning.columns_with_no_statistics[0].column, "Value");

    // Multi-thread runtime stats on table scan
    let stats = table_scan
        .runtime_stats
        .as_ref()
        .expect("should have runtime stats");
    assert_eq!(stats.actual_rows, 1000); // 500 + 500
    assert_eq!(stats.actual_executions, 2); // 1 + 1
    assert_eq!(stats.actual_elapsed_ms, Some(12)); // max(10, 12)
    assert_eq!(stats.actual_cpu_ms, Some(17)); // 8 + 9
    assert_eq!(stats.actual_scans, Some(2));
    assert_eq!(stats.actual_logical_reads, Some(200)); // 100 + 100
    assert_eq!(stats.actual_physical_reads, Some(18)); // 10 + 8
    assert_eq!(stats.actual_read_aheads, Some(90)); // 50 + 40
}

// -----------------------------------------------------------------------
// Memory grant info and wait stats
// -----------------------------------------------------------------------

#[test]
fn parse_memory_grant_and_wait_stats() {
    let xml = include_str!("../tests/data/memory_grant_wait_stats_plan.xml");
    let plan = parse(xml).unwrap();

    // Memory grant
    let mg = plan
        .memory_grant
        .as_ref()
        .expect("should have memory grant");
    assert_eq!(mg.serial_required_memory, 1024);
    assert_eq!(mg.serial_desired_memory, 4096);
    assert_eq!(mg.required_memory, Some(2048));
    assert_eq!(mg.desired_memory, Some(8192));
    assert_eq!(mg.requested_memory, Some(8192));
    assert_eq!(mg.granted_memory, Some(6144));
    assert_eq!(mg.max_used_memory, Some(5120));
    assert_eq!(mg.grant_wait_time, Some(150));

    // Wait stats
    assert_eq!(plan.wait_stats.len(), 3);
    assert_eq!(plan.wait_stats[0].wait_type, "CXPACKET");
    assert_eq!(plan.wait_stats[0].wait_time_ms, 1234);
    assert_eq!(plan.wait_stats[0].wait_count, 56);
    assert_eq!(plan.wait_stats[1].wait_type, "PAGEIOLATCH_SH");
    assert_eq!(plan.wait_stats[1].wait_time_ms, 567);
    assert_eq!(plan.wait_stats[2].wait_type, "SOS_SCHEDULER_YIELD");
    assert_eq!(plan.wait_stats[2].wait_count, 100);

    // Non-parallel plan reason
    assert_eq!(
        plan.non_parallel_plan_reason.as_deref(),
        Some("CouldNotGenerateValidParallelPlan")
    );

    // DOP
    assert_eq!(plan.degree_of_parallelism, Some(2));

    // New RelOp attributes
    let root = &plan.root;
    assert_eq!(root.estimated_execution_mode.as_deref(), Some("Row"));

    let child = &root.children[0];
    assert_eq!(child.partitioned, Some(true));
    assert_eq!(child.estimated_rows_read, Some(100000.0));

    // Parameter list with compiled vs runtime values
    assert_eq!(plan.parameters.len(), 1);
    assert_eq!(plan.parameters[0].column, "@Cat");
    assert_eq!(
        plan.parameters[0].parameter_data_type.as_deref(),
        Some("nvarchar(50)")
    );
    assert_eq!(
        plan.parameters[0].parameter_compiled_value.as_deref(),
        Some("N'Electronics'")
    );
    assert_eq!(
        plan.parameters[0].parameter_runtime_value.as_deref(),
        Some("N'Books'")
    );
}

// -----------------------------------------------------------------------
// QueryPlan-level warnings (NoJoinPredicate at QueryPlan level)
// -----------------------------------------------------------------------

#[test]
fn parse_query_plan_level_warnings() {
    let xml = include_str!("../tests/data/warnings_plan.xml");
    let plan = parse(xml).unwrap();

    // The <QueryPlan><Warnings NoJoinPredicate="true" /> is a query-plan level warning.
    // Since it appears before any RelOp is opened, it's attached to the sentinel
    // (which becomes the parent). The sentinel's warnings end up merged into the root.
    // In our parser, it actually attaches to the sentinel which wraps everything.
    // Let's verify the root has warnings with no_join_predicate set.
    let has_njp = plan.root.warnings.iter().any(|w| w.no_join_predicate);
    assert!(has_njp, "should have no_join_predicate warning");
}

// -----------------------------------------------------------------------
// Statement metadata from StmtSimple
// -----------------------------------------------------------------------

#[test]
fn parse_statement_metadata() {
    let xml = include_str!("../tests/data/estimated_plan.xml");
    let plan = parse(xml).unwrap();

    assert_eq!(plan.statement_type.as_deref(), Some("SELECT"));
    assert_eq!(plan.optimization_level.as_deref(), Some("FULL"));
    assert_eq!(
        plan.early_abort_reason.as_deref(),
        Some("GoodEnoughPlanFound")
    );
    assert_eq!(plan.cardinality_estimation_model.as_deref(), Some("150"));
    assert_eq!(plan.query_plan_hash.as_deref(), Some("0x1234567890ABCDEF"));
}

// -----------------------------------------------------------------------
// FromStr / TryFrom conversions
// -----------------------------------------------------------------------

#[test]
fn from_str_single_statement() {
    use crate::ShowPlan;
    let xml = include_str!("../tests/data/estimated_plan.xml");
    let plan: ShowPlan = xml.parse().expect("FromStr should succeed");
    assert!(!plan.root.physical_op.is_empty());
}

#[test]
fn try_from_str_single_statement() {
    use crate::ShowPlan;
    let xml = include_str!("../tests/data/estimated_plan.xml");
    let plan = ShowPlan::try_from(xml).expect("TryFrom<&str> should succeed");
    assert!(!plan.root.physical_op.is_empty());
}

#[test]
fn try_from_owned_string() {
    use crate::ShowPlan;
    let xml: String = include_str!("../tests/data/estimated_plan.xml").to_owned();
    let plan = ShowPlan::try_from(xml).expect("TryFrom<String> should succeed");
    assert!(!plan.root.physical_op.is_empty());
}

#[test]
fn from_str_batch_wraps_parse_all() {
    use crate::ShowPlanBatch;
    let xml = include_str!("../tests/data/estimated_plan.xml");
    let batch: ShowPlanBatch = xml.parse().expect("FromStr should succeed");
    assert!(!batch.is_empty());
    assert_eq!(batch.len(), batch.as_slice().len());
}

#[test]
fn try_from_batch_iteration() {
    use crate::ShowPlanBatch;
    let xml = include_str!("../tests/data/estimated_plan.xml");
    let batch = ShowPlanBatch::try_from(xml).unwrap();
    let count = (&batch).into_iter().count();
    assert_eq!(count, batch.len());
    let plans: Vec<_> = batch.into_inner();
    assert!(!plans.is_empty());
}

#[test]
fn from_str_multi_statement_errors_for_showplan() {
    use crate::{Error, ShowPlan};
    let xml = r#"<?xml version="1.0"?>
<ShowPlanXML xmlns="http://schemas.microsoft.com/sqlserver/2004/07/showplan">
  <BatchSequence><Batch><Statements>
    <StmtSimple StatementText="SELECT 1">
      <QueryPlan><RelOp NodeId="1" PhysicalOp="Constant Scan" LogicalOp="Constant Scan"
        EstimateRows="1" EstimateCPU="0" EstimateIO="0"
        EstimatedTotalSubtreeCost="0" Parallel="false"/></QueryPlan>
    </StmtSimple>
    <StmtSimple StatementText="SELECT 2">
      <QueryPlan><RelOp NodeId="1" PhysicalOp="Constant Scan" LogicalOp="Constant Scan"
        EstimateRows="1" EstimateCPU="0" EstimateIO="0"
        EstimatedTotalSubtreeCost="0" Parallel="false"/></QueryPlan>
    </StmtSimple>
  </Statements></Batch></BatchSequence>
</ShowPlanXML>"#;
    let err = xml.parse::<ShowPlan>().unwrap_err();
    assert!(matches!(err, Error::MultipleStatements(2)));

    // ShowPlanBatch handles it just fine.
    let batch: crate::ShowPlanBatch = xml.parse().unwrap();
    assert_eq!(batch.len(), 2);
}
