# sqldev-showplan

Parse SQL Server `SHOWPLAN_XML` and `STATISTICS XML` output into a typed Rust plan tree.

## Features

- **Estimated plans** (`SET SHOWPLAN_XML ON`) — parses `<RelOp>` nodes with cost estimates
- **Actual plans** (`SET STATISTICS XML ON`) — includes `ActualRows`, `ActualExecutions`, and runtime stats
- **Plan metadata** — statement text, query hash, cached plan size, compile time/CPU/memory
- **Feature-gated serde** — enable the `serde` feature for `Serialize`/`Deserialize` derives
- **Zero-copy XML parsing** via `quick-xml`

## Quick start

```rust
use sqldev_showplan::parse;

let xml = r#"<?xml version="1.0"?>
<ShowPlanXML xmlns="http://schemas.microsoft.com/sqlserver/2004/07/showplan">
  <BatchSequence>
    <Batch>
      <Statements>
        <StmtSimple StatementText="SELECT * FROM Users"
                    QueryHash="0xABC123">
          <QueryPlan CachedPlanSize="32"
                     CompileTime="5"
                     CompileCPU="3"
                     CompileMemory="128">
            <RelOp NodeId="1"
                   PhysicalOp="Clustered Index Scan"
                   LogicalOp="Clustered Index Scan"
                   EstimateRows="100"
                   EstimateCPU="0.001"
                   EstimateIO="0.01"
                   EstimatedTotalSubtreeCost="0.011"
                   Parallel="false" />
          </QueryPlan>
        </StmtSimple>
      </Statements>
    </Batch>
  </BatchSequence>
</ShowPlanXML>"#;

let plan = parse(xml).expect("valid plan");
assert_eq!(plan.root.physical_op, "Clustered Index Scan");
assert_eq!(plan.statement_text, Some("SELECT * FROM Users".into()));
```

## Cargo features

| Feature | Default | Description |
|---------|---------|-------------|
| `serde` | no | Adds `Serialize` / `Deserialize` to all public types |

## License

Licensed under either of [Apache License, Version 2.0](../../LICENSE-APACHE) or
[MIT License](../../LICENSE-MIT) at your option.
