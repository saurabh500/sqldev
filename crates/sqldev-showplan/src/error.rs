//! Error types for the `sqldev-showplan` crate.

/// Errors that can occur while parsing a SQL Server execution plan.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The XML could not be parsed.
    #[error("XML parse error: {0}")]
    Xml(#[from] quick_xml::Error),

    /// The plan XML contained no `<RelOp>` elements.
    #[error("execution plan is empty — no <RelOp> elements found")]
    EmptyPlan,

    /// `parse` was called on XML that contained more than one statement.
    /// Use [`crate::parse_all`] to retrieve all of them.
    #[error("execution plan contains {0} statements; use parse_all() to retrieve all of them")]
    MultipleStatements(usize),
}
