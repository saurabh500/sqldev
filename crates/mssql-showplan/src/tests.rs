use crate::{parse, PlanNode, ShowPlan};

/// Helper to count all nodes in the tree.
fn count_nodes(node: &PlanNode) -> usize {
    1 + node.children.iter().map(count_nodes).sum::<usize>()
}

// -----------------------------------------------------------------------
// Estimated plan (multi-node join + sort)
// -----------------------------------------------------------------------

#[test]
fn parse_estimated_plan() {
    let xml = include_str!("data/estimated_plan.xml");
    let plan = parse(xml).expect("should parse estimated plan");

    // Metadata
    assert!(plan
        .statement_text
        .as_deref()
        .unwrap()
        .contains("SELECT o.OrderID"));
    assert_eq!(
        plan.query_hash.as_deref(),
        Some("0xA1B2C3D4E5F60718")
    );
    assert_eq!(plan.cached_plan_size, Some(48));
    assert_eq!(plan.compile_time, Some(12));
    assert_eq!(plan.compile_cpu, Some(8));
    assert_eq!(plan.compile_memory, Some(256));

    // Root operator
    assert_eq!(plan.root.physical_op, "Sort");
    assert_eq!(plan.root.logical_op, "Sort");
    assert_eq!(plan.root.node_id, 0);
    assert!(!plan.root.parallel);

    // Cost estimates on root
    assert!((plan.root.estimated_rows.unwrap() - 500.0).abs() < f64::EPSILON);
    assert!((plan.root.estimated_total_subtree_cost.unwrap() - 1.234).abs() < 1e-6);

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
}

// -----------------------------------------------------------------------
// Actual plan (STATISTICS XML with RunTimeCountersPerThread)
// -----------------------------------------------------------------------

#[test]
fn parse_actual_plan() {
    let xml = include_str!("data/actual_plan.xml");
    let plan = parse(xml).expect("should parse actual plan");

    assert_eq!(
        plan.query_hash.as_deref(),
        Some("0xFEDCBA9876543210")
    );

    // Root is Top
    assert_eq!(plan.root.physical_op, "Top");
    assert_eq!(plan.root.children.len(), 1);

    // Child has runtime stats
    let scan = &plan.root.children[0];
    assert_eq!(scan.physical_op, "Clustered Index Scan");
    assert_eq!(scan.actual_rows, Some(10.0));
    assert_eq!(scan.actual_executions, Some(1));
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
               EstimatedTotalSubtreeCost="5.0" Parallel="true">
          <RelOp NodeId="1" PhysicalOp="Clustered Index Scan" LogicalOp="Clustered Index Scan"
                 EstimateRows="1000000" EstimateCPU="1.0" EstimateIO="3.5"
                 EstimatedTotalSubtreeCost="4.5" Parallel="true" />
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
    assert!(msg.contains("empty"), "Error should mention empty plan: {msg}");
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
               EstimatedTotalSubtreeCost="0.0001" Parallel="false"
               TableCardinality="42" />
      </QueryPlan>
    </StmtSimple>
  </Statements></Batch></BatchSequence>
</ShowPlanXML>"#;

    let plan = parse(xml).unwrap();
    assert!(plan.root.extra.contains_key("TableCardinality"));
    assert_eq!(plan.root.extra["TableCardinality"], serde_json::json!(42.0));
}

// -----------------------------------------------------------------------
// serde round-trip (compile-time check that derives are present)
// -----------------------------------------------------------------------

#[cfg(feature = "serde")]
#[test]
fn serde_round_trip() {
    let xml = include_str!("data/estimated_plan.xml");
    let plan = parse(xml).unwrap();
    let json = serde_json::to_string_pretty(&plan).expect("serialize");
    let deser: ShowPlan = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(deser.root.physical_op, plan.root.physical_op);
    assert_eq!(deser.query_hash, plan.query_hash);
}
