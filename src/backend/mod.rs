//! Executing SQL on a database.

pub mod postgres;

use crate::Result;

/// A value read from a result row.
#[derive(Debug, Clone, PartialEq)]
pub enum SqlValue {
    /// SQL `NULL`.
    Null,
    /// A boolean.
    Bool(bool),
    /// A 64-bit integer.
    Long(i64),
    /// A string.
    Text(String),
    /// A JSON document.
    Json(serde_json::Value),
}

/// One result row.
pub type Row = Vec<SqlValue>;

/// A database connection this crate can run its statements on.
pub trait Backend {
    /// Runs one or more statements, separated by `;`, without parameters.
    fn execute_batch(&mut self, sql: &str) -> Result<()>;
    /// Runs one query and returns its rows.
    fn query(&mut self, sql: &str) -> Result<Vec<Row>>;
    /// Starts a transaction.
    fn begin(&mut self) -> Result<()> {
        self.execute_batch("BEGIN")
    }
    /// Discards the current transaction.
    fn rollback(&mut self) -> Result<()> {
        self.execute_batch("ROLLBACK")
    }
}
