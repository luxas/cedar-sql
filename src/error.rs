//! The crate's error type.

/// A `Result` with this crate's [`Error`].
pub type Result<T> = std::result::Result<T, Error>;

/// Why an operation of this crate failed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// A construct this crate does not compile (yet). The differential tests
    /// treat this as a benign skip, so the string names the construct.
    #[error("unsupported: {0}")]
    Unsupported(&'static str),
    /// The database returned an error.
    #[error("database error: {0}")]
    Database(#[from] postgres::Error),
    /// A row's column had a type or value this crate cannot decode.
    #[error("cannot decode column {column}: {message}")]
    Decode {
        /// The column's index in the row.
        column: usize,
        /// What went wrong.
        message: String,
    },
    /// The test-only database provisioning failed (feature `testing`).
    #[error("cannot provision a Postgres for testing: {0}")]
    Provision(String),
}
