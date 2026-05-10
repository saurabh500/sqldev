//! Error types for the `mssql-showplan` crate.

/// Errors that can occur while parsing a SQL Server execution plan.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The XML could not be parsed.
    #[error("XML parse error: {0}")]
    Xml(#[from] quick_xml::Error),

    /// The plan XML contained no `<RelOp>` elements.
    #[error("execution plan is empty — no <RelOp> elements found")]
    EmptyPlan,
}
