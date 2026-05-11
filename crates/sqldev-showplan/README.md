# sqldev-showplan

Parse SQL Server `SHOWPLAN_XML` and `STATISTICS XML` output into a typed Rust plan tree.

## Features

- **Estimated plans** (`SET SHOWPLAN_XML ON`) — parses `<RelOp>` nodes with cost estimates
- **Actual plans** (`SET STATISTICS XML ON`) — includes `ActualRows`, `ActualExecutions`, and runtime stats
- **Plan metadata** — statement text, query hash, cached plan size, compile time/CPU/memory
- **Multi-statement batches** — use [`parse_all`] to retrieve one [`ShowPlan`] per statement
- **Predicate / Object capture** — residual predicates and base table/index references attached to each operator
- **Feature-gated serde** — enable the `serde` feature for `Serialize`/`Deserialize` derives
- **Zero-copy XML parsing** via `quick-xml`

## Encoding

SQL Server returns SHOWPLAN XML as **UTF‑16** over the TDS protocol. If you read
the raw bytes (e.g. through `tiberius`), decode them to a UTF‑8 `String` before
passing to `parse` / `parse_all`.

## Quick start

```rust
use sqldev_showplan::ShowPlan;

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

// Idiomatic: use `FromStr` / `TryFrom`
let plan: ShowPlan = xml.parse().expect("valid plan");
assert_eq!(plan.root.physical_op, "Clustered Index Scan");

// For multi-statement batches:
use sqldev_showplan::ShowPlanBatch;
let batch: ShowPlanBatch = xml.parse().expect("valid batch");
for stmt in &batch {
    println!("{}", stmt.root.physical_op);
}
```

The free functions `parse(xml)` and `parse_all(xml)` are also exposed for
discoverability and ergonomic use without turbofish.

## Cargo features

| Feature | Default | Description |
|---------|---------|-------------|
| `serde` | no | Adds `Serialize` / `Deserialize` to all public types |

## License

Licensed under either of [Apache License, Version 2.0](../../LICENSE-APACHE) or
[MIT License](../../LICENSE-MIT) at your option.
