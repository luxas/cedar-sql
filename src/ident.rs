//! SQL identifiers.
//!
//! Plan 2 adds `SQLIdentifier`: a validated identifier of at most 63 bytes whose
//! `Display` emits the double-quoted form with `"` doubled, plus
//! `quoted_literal` for single-quoted string literals.
