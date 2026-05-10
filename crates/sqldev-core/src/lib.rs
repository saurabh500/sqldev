//! Shared types for the `sqldev` workspace.
//!
//! Currently this crate holds:
//! - [`schema`] — the v0.1 schema graph contract (validated against
//!   `AdventureWorks2022` in the M0 spikes).
//! - [`error`] — typed top-level error.
//!
//! It deliberately does no I/O. Network calls live in `sqldev-conn`,
//! catalog walking in `sqldev-introspect`.

#![forbid(unsafe_code)]

pub mod error;
pub mod schema;

pub use error::{Error, Result};
pub use schema::{
    CheckConstraint, Column, ForeignKey, Index, KeyConstraint, Routine, SchemaGraph, SchemaNode,
    Table, Trigger, UserDefinedType, View,
};
