//! Compiling a TPE residual into a SQL expression.
//!
//! Plan 3 adds the three-valued encoding (`NULL` is an error) for the core
//! operators; Plan 4 completes it with sets, records, `like`, tags and
//! memoization.
