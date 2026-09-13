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
    /// The policies do not validate against the schema (strict mode), which
    /// the compilation requires.
    #[error("the policies do not validate: {0}")]
    Validation(String),
    /// The request is not valid for the schema, or is not concrete enough.
    #[error("invalid request: {0}")]
    Request(String),
    /// Partial evaluation failed.
    #[error("partial evaluation failed: {0}")]
    Tpe(String),
    /// The query returned rows this crate cannot interpret.
    #[error("unexpected query result: {0}")]
    Query(String),
    /// A string holds a NUL character, which Postgres `text` cannot store: a
    /// documented limitation, distinguishable from a loader bug.
    #[error("the string {0:?} contains a NUL character, which Postgres cannot store")]
    Nul(String),
    /// The entities cannot be turned into rows of the configured tables.
    #[error("cannot load entities: {0}")]
    Load(String),
    /// The database returned an error.
    #[error("database error: {}", describe(.0))]
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

impl Error {
    /// Whether the database cancelled the statement (a `statement_timeout`).
    pub fn is_timeout(&self) -> bool {
        match self {
            Error::Database(e) => e
                .as_db_error()
                .is_some_and(|db| *db.code() == postgres::error::SqlState::QUERY_CANCELED),
            _ => false,
        }
    }
}

/// The server's message when there is one (`postgres::Error`'s own `Display`
/// is only "db error"), else the client-side description.
fn describe(error: &postgres::Error) -> String {
    match error.as_db_error() {
        Some(db) => format!(
            "{} ({}){}",
            db.message(),
            db.code().code(),
            db.detail().map(|d| format!(": {d}")).unwrap_or_default()
        ),
        None => error.to_string(),
    }
}
