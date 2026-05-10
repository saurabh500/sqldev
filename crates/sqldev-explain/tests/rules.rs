//! Per-rule fixture tests. Each fixture is a minimal hand-crafted
//! showplan-XML snippet exercising exactly one rule.

use sqldev_explain::{Severity, analyze_plan};

fn rules(xml: &str) -> Vec<&'static str> {
    analyze_plan(xml)
        .unwrap()
        .into_iter()
        .map(|f| f.rule)
        .collect()
}

#[test]
fn clean_plan_emits_nothing() {
    let xml = wrap_stmt(
        "SELECT 1",
        r#"<RelOp PhysicalOp="Constant Scan" LogicalOp="Constant Scan" EstimateRows="1" EstimatedTotalSubtreeCost="0.001"/>"#,
    );
    assert!(analyze_plan(&xml).unwrap().is_empty());
}

#[test]
fn detects_table_scan_no_index() {
    let xml = wrap_stmt(
        "SELECT * FROM Heap WHERE x = 1",
        r#"<RelOp PhysicalOp="Table Scan" LogicalOp="Table Scan" EstimateRows="500" TableCardinality="100000" EstimatedTotalSubtreeCost="3.5">
            <TableScan>
              <Object Schema="[dbo]" Table="[Heap]"/>
              <Predicate><ScalarOperator/></Predicate>
            </TableScan>
          </RelOp>"#,
    );
    assert!(rules(&xml).contains(&"table-scan-no-index"));
}

#[test]
fn detects_clustered_index_scan_with_residual() {
    let xml = wrap_stmt(
        "SELECT * FROM T WHERE y = 1",
        r#"<RelOp PhysicalOp="Clustered Index Scan" LogicalOp="Clustered Index Scan" EstimateRows="800" TableCardinality="200000" EstimatedTotalSubtreeCost="4.2">
            <IndexScan>
              <Object Schema="[dbo]" Table="[T]" Index="[PK_T]"/>
              <Predicate><ScalarOperator/></Predicate>
            </IndexScan>
          </RelOp>"#,
    );
    assert!(rules(&xml).contains(&"clustered-index-scan-with-residual"));
}

#[test]
fn detects_key_lookup() {
    let xml = wrap_stmt(
        "SELECT a,b,c FROM T WHERE a = 1",
        r#"<RelOp PhysicalOp="Nested Loops" LogicalOp="Inner Join" EstimateRows="10" EstimatedTotalSubtreeCost="0.5">
            <RelOp PhysicalOp="Index Seek" LogicalOp="Index Seek" EstimateRows="10" EstimatedTotalSubtreeCost="0.1">
              <IndexScan>
                <Object Schema="[dbo]" Table="[T]" Index="[IX_a]"/>
              </IndexScan>
            </RelOp>
            <RelOp PhysicalOp="Clustered Index Seek" LogicalOp="Clustered Index Seek" EstimateRows="1" EstimatedTotalSubtreeCost="0.4">
              <IndexScan Lookup="1">
                <Object Schema="[dbo]" Table="[T]" Index="[PK_T]"/>
              </IndexScan>
            </RelOp>
          </RelOp>"#,
    );
    let rs = rules(&xml);
    assert!(
        rs.contains(&"key-lookup-on-non-covering-index"),
        "got {rs:?}"
    );
}

#[test]
fn detects_rid_lookup() {
    let xml = wrap_stmt(
        "SELECT * FROM Heap WHERE k=1",
        r#"<RelOp PhysicalOp="RID Lookup" LogicalOp="RID Lookup" EstimateRows="1" EstimatedTotalSubtreeCost="0.3">
            <IndexScan Lookup="1">
              <Object Schema="[dbo]" Table="[Heap]"/>
            </IndexScan>
          </RelOp>"#,
    );
    assert!(rules(&xml).contains(&"rid-lookup"));
}

#[test]
fn detects_implicit_conversion_on_column() {
    let xml = wrap_stmt(
        "SELECT * FROM T WHERE name = 'foo'",
        r#"<RelOp PhysicalOp="Index Scan" LogicalOp="Index Scan" EstimateRows="10" TableCardinality="10000" EstimatedTotalSubtreeCost="2.0">
            <IndexScan>
              <Object Schema="[dbo]" Table="[T]" Index="[IX_T]"/>
              <Predicate>
                <ScalarOperator>
                  <Convert DataType="nvarchar" Implicit="1">
                    <ScalarOperator>
                      <Identifier><ColumnReference Schema="[dbo]" Table="[T]" Column="name"/></Identifier>
                    </ScalarOperator>
                  </Convert>
                </ScalarOperator>
              </Predicate>
            </IndexScan>
          </RelOp>"#,
    );
    assert!(rules(&xml).contains(&"implicit-conversion-on-column"));
}

#[test]
fn detects_hash_spill() {
    let xml = wrap_stmt(
        "SELECT a,b FROM T GROUP BY a,b",
        r#"<RelOp PhysicalOp="Hash Match" LogicalOp="Aggregate" EstimateRows="50000" EstimatedTotalSubtreeCost="22.0">
            <Hash>
              <Warnings><SpillToTempDb SpillLevel="2"/></Warnings>
            </Hash>
          </RelOp>"#,
    );
    assert!(rules(&xml).contains(&"hash-spill-to-tempdb"));
}

#[test]
fn detects_sort_warning() {
    let xml = wrap_stmt(
        "SELECT * FROM T ORDER BY x",
        r#"<RelOp PhysicalOp="Sort" LogicalOp="Sort" EstimateRows="500000" EstimatedTotalSubtreeCost="40.0">
            <Sort>
              <Warnings><SortWarning Description="Sort spilled"/></Warnings>
            </Sort>
          </RelOp>"#,
    );
    assert!(rules(&xml).contains(&"sort-warning"));
}

#[test]
fn detects_parameter_sniffing_tell() {
    let xml = wrap_stmt(
        "SELECT * FROM T WHERE k=@p ORDER BY x",
        r#"<RelOp PhysicalOp="Sort" LogicalOp="Sort" EstimateRows="20" EstimatedTotalSubtreeCost="2.0">
            <RelOp PhysicalOp="Index Seek" LogicalOp="Index Seek" EstimateRows="20" TableCardinality="100000" EstimatedTotalSubtreeCost="1.0">
              <IndexScan>
                <Object Schema="[dbo]" Table="[T]" Index="[IX]"/>
              </IndexScan>
            </RelOp>
          </RelOp>"#,
    );
    assert!(rules(&xml).contains(&"parameter-sniffing-tell"));
}

#[test]
fn detects_missing_index_hint() {
    let xml = wrap_plan(
        "SELECT * FROM T WHERE x=1",
        r#"<MissingIndexes>
             <MissingIndexGroup Impact="86.4">
               <MissingIndex Database="[db]" Schema="[dbo]" Table="[T]">
                 <ColumnGroup Usage="EQUALITY"><Column Name="[x]" ColumnId="1"/></ColumnGroup>
               </MissingIndex>
             </MissingIndexGroup>
           </MissingIndexes>
           <RelOp PhysicalOp="Clustered Index Scan" LogicalOp="Clustered Index Scan" EstimateRows="1" TableCardinality="50" EstimatedTotalSubtreeCost="0.01"/>"#,
        "",
    );
    let findings = analyze_plan(&xml).unwrap();
    let hit = findings
        .iter()
        .find(|f| f.rule == "missing-index-hint")
        .unwrap();
    assert_eq!(hit.severity, Severity::Warning);
    assert!(hit.est_cost_delta.is_some());
}

#[test]
fn detects_parallelism_cost_warning() {
    let xml = r#"<ShowPlanXML xmlns="http://schemas.microsoft.com/sqlserver/2004/07/showplan">
              <BatchSequence><Batch><Statements>
                <StmtSimple StatementText="SELECT 1">
                  <QueryPlan DegreeOfParallelism="4">
                    <RelOp PhysicalOp="Hash Match" LogicalOp="Inner Join" EstimateRows="100" EstimatedTotalSubtreeCost="42.0"/>
                  </QueryPlan>
                </StmtSimple>
              </Statements></Batch></BatchSequence>
            </ShowPlanXML>"#;
    assert!(rules(xml).contains(&"parallelism-cost-warning"));
}

#[test]
fn detects_excessive_memory_grant() {
    let xml = format!(
        r#"<ShowPlanXML xmlns="http://schemas.microsoft.com/sqlserver/2004/07/showplan">
              <BatchSequence><Batch><Statements>
                <StmtSimple StatementText="SELECT *">
                  <QueryPlan SerialDesiredMemory="{}">
                    <RelOp PhysicalOp="Hash Match" LogicalOp="Aggregate" EstimateRows="50" EstimatedTotalSubtreeCost="3.0"/>
                  </QueryPlan>
                </StmtSimple>
              </Statements></Batch></BatchSequence>
            </ShowPlanXML>"#,
        800 * 1024 // 800 MB in KB
    );
    assert!(rules(&xml).contains(&"excessive-memory-grant"));
}

#[test]
fn detects_nested_loops_large_outer() {
    let xml = wrap_stmt(
        "SELECT * FROM A JOIN B",
        r#"<RelOp PhysicalOp="Nested Loops" LogicalOp="Inner Join" EstimateRows="50000" EstimatedTotalSubtreeCost="120.0">
            <RelOp PhysicalOp="Index Scan" LogicalOp="Index Scan" EstimateRows="50000" EstimatedTotalSubtreeCost="10.0"/>
            <RelOp PhysicalOp="Index Seek" LogicalOp="Index Seek" EstimateRows="1" EstimatedTotalSubtreeCost="0.1"/>
          </RelOp>"#,
    );
    assert!(rules(&xml).contains(&"nested-loops-on-large-outer"));
}

#[test]
fn detects_eager_spool() {
    let xml = wrap_stmt(
        "UPDATE T SET x = x+1",
        r#"<RelOp PhysicalOp="Eager Spool" LogicalOp="Eager Spool" EstimateRows="200" EstimatedTotalSubtreeCost="5.0"/>"#,
    );
    assert!(rules(&xml).contains(&"eager-spool-warning"));
}

#[test]
fn detects_udf_in_predicate() {
    let xml = wrap_stmt(
        "SELECT * FROM T WHERE dbo.IsActive(id) = 1",
        r#"<RelOp PhysicalOp="Clustered Index Scan" LogicalOp="Clustered Index Scan" EstimateRows="1000" TableCardinality="100000" EstimatedTotalSubtreeCost="3.0">
            <IndexScan>
              <Object Schema="[dbo]" Table="[T]" Index="[PK]"/>
              <Predicate>
                <ScalarOperator>
                  <UserDefinedFunction FunctionName="[dbo].[IsActive]"/>
                </ScalarOperator>
              </Predicate>
            </IndexScan>
          </RelOp>"#,
    );
    assert!(rules(&xml).contains(&"udf-in-predicate"));
}

#[test]
fn detects_join_without_statistics() {
    let xml = wrap_plan("SELECT * FROM A JOIN B ON A.x=B.y", "", "");
    // Inject ColumnsWithNoStatistics inside the StmtSimple.
    let xml = xml.replace(
        "<RelOp PhysicalOp=\"Constant Scan\"",
        r#"<ColumnsWithNoStatistics>
             <ColumnReference Schema="[dbo]" Table="[B]" Column="y"/>
           </ColumnsWithNoStatistics>
           <RelOp PhysicalOp="Constant Scan""#,
    );
    assert!(rules(&xml).contains(&"join-without-statistics"));
}

#[test]
fn isolates_showplan_from_sqlcmd_transcript() {
    let xml = wrap_stmt(
        "SELECT 1",
        r#"<RelOp PhysicalOp="Constant Scan" LogicalOp="Constant Scan" EstimateRows="1" EstimatedTotalSubtreeCost="0.0"/>"#,
    );
    let transcript = format!("Microsoft (R) ... \n{xml}\n(1 row affected)\n");
    let findings = analyze_plan(&transcript).unwrap();
    assert!(findings.is_empty());
}

// -- helpers ---------------------------------------------------------------

fn wrap_stmt(stmt_text: &str, inner: &str) -> String {
    wrap_plan(stmt_text, "", inner)
}

/// Build a `ShowPlanXML` doc. `extras_inside_stmt` is injected immediately
/// before the `QueryPlan`; `query_plan_attrs` adds attributes to `QueryPlan`
/// (e.g. `DegreeOfParallelism="4"`).
fn wrap_plan(
    stmt_text: &str,
    extras_inside_stmt: &str,
    query_plan_attrs_or_root_inner: &str,
) -> String {
    // For tests that pass a QueryPlan attribute string (e.g. `DegreeOfParallelism="4"`),
    // detect by leading `Degree`/`Serial` letters; otherwise treat as root RelOp inner.
    let is_attrs = query_plan_attrs_or_root_inner
        .trim_start()
        .starts_with("Degree")
        || query_plan_attrs_or_root_inner
            .trim_start()
            .starts_with("Serial");
    let (attrs, root) = if is_attrs {
        (query_plan_attrs_or_root_inner, "<RelOp PhysicalOp=\"Constant Scan\" LogicalOp=\"Constant Scan\" EstimateRows=\"1\" EstimatedTotalSubtreeCost=\"0.0\"/>".to_string())
    } else {
        ("", query_plan_attrs_or_root_inner.to_string())
    };
    let root = if root.is_empty() {
        "<RelOp PhysicalOp=\"Constant Scan\" LogicalOp=\"Constant Scan\" EstimateRows=\"1\" EstimatedTotalSubtreeCost=\"0.0\"/>".to_string()
    } else {
        root
    };
    format!(
        r#"<ShowPlanXML xmlns="http://schemas.microsoft.com/sqlserver/2004/07/showplan">
              <BatchSequence><Batch><Statements>
                <StmtSimple StatementText="{stmt_text}">
                  {extras_inside_stmt}
                  <QueryPlan {attrs}>
                    {root}
                  </QueryPlan>
                </StmtSimple>
              </Statements></Batch></BatchSequence>
            </ShowPlanXML>"#
    )
}
