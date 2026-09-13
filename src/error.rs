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
    /// A string is not a valid SQL identifier.
    #[error("invalid SQL identifier {0:?}: {1}")]
    Identifier(String, &'static str),
    /// The Cedar schema cannot be mapped to a database configuration.
    #[error("cannot map the schema to SQL: {0}")]
    Schema(String),
    /// An `@sql_*` annotation is malformed.
    #[error("invalid @{annotation} annotation on {on}: {message}")]
    Annotation {
        /// The annotation key, without the `@`.
        annotation: &'static str,
        /// The declaration the annotation is on.
        on: String,
        /// What is wrong with it.
        message: String,
    },
    /// The entities cannot be turned into rows of the configured tables.
    #[error("cannot load entities: {0}")]
    Load(String),
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
