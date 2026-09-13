//! SQL identifiers and literals.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::{Error, Result};

/// The longest identifier Postgres keeps intact (`NAMEDATALEN - 1`).
pub const MAX_IDENTIFIER_BYTES: usize = 63;

/// A validated SQL identifier: non-empty, at most [`MAX_IDENTIFIER_BYTES`]
/// bytes, without NUL characters. `Display` renders the double-quoted form,
/// so any identifier — a Cedar type name such as `App::User` included — is
/// safe to splice into a statement.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct SQLIdentifier(String);

impl SQLIdentifier {
    /// Validates `name` as an identifier.
    pub fn new(name: impl Into<String>) -> Result<Self> {
        let name = name.into();
        if name.is_empty() {
            return Err(Error::Identifier(name, "empty"));
        }
        if name.len() > MAX_IDENTIFIER_BYTES {
            return Err(Error::Identifier(name, "longer than 63 bytes"));
        }
        if name.contains('\0') {
            return Err(Error::Identifier(name, "contains a NUL character"));
        }
        Ok(Self(name))
    }

    /// The identifier's text, unquoted.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// `name` followed by `suffix`, e.g. for the `__entity_id2` naming rule or
    /// a `_tags` table.
    pub fn with_suffix(&self, suffix: &str) -> Result<Self> {
        Self::new(format!("{}{suffix}", self.0))
    }
}

impl std::borrow::Borrow<str> for SQLIdentifier {
    fn borrow(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SQLIdentifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "\"{}\"", self.0.replace('"', "\"\""))
    }
}

impl TryFrom<String> for SQLIdentifier {
    type Error = Error;

    fn try_from(name: String) -> Result<Self> {
        Self::new(name)
    }
}

impl From<SQLIdentifier> for String {
    fn from(ident: SQLIdentifier) -> String {
        ident.0
    }
}

/// `raw` as an identifier, shortened with a hash of the whole name when it is
/// longer than [`MAX_IDENTIFIER_BYTES`] (Postgres would otherwise truncate it
/// silently, so that two long names could collide). NUL characters are
/// dropped.
pub fn shortened(raw: &str) -> SQLIdentifier {
    let raw = raw.replace('\0', "");
    if !raw.is_empty() && raw.len() <= MAX_IDENTIFIER_BYTES {
        return SQLIdentifier::new(raw).expect("validated");
    }
    let hash = format!("{:016x}", fnv1a(&raw));
    let mut cut = (MAX_IDENTIFIER_BYTES - hash.len() - 1).min(raw.len());
    while !raw.is_char_boundary(cut) {
        cut -= 1;
    }
    SQLIdentifier::new(format!("{}_{hash}", &raw[..cut])).expect("within the limit")
}

/// A small stable hash (FNV-1a), so generated names do not depend on the
/// standard library's hasher.
fn fnv1a(s: &str) -> u64 {
    s.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// `s` as a single-quoted SQL string literal, with `'` doubled. Backslashes
/// are literal (Postgres `standard_conforming_strings`, SQLite always).
///
/// # Errors
///
/// When `s` contains a NUL character, which Postgres `text` cannot store.
pub fn quoted_literal(s: &str) -> Result<String> {
    if s.contains('\0') {
        return Err(Error::Load(format!(
            "the string {s:?} contains a NUL character, which Postgres cannot store"
        )));
    }
    Ok(format!("'{}'", s.replace('\'', "''")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting() {
        let id = SQLIdentifier::new("App::\"User\"").unwrap();
        assert_eq!(id.to_string(), "\"App::\"\"User\"\"\"");
        assert_eq!(quoted_literal("it's").unwrap(), "'it''s'");
        assert!(quoted_literal("a\0b").is_err());
    }

    #[test]
    fn shortening() {
        let long = format!("{}{}", "a".repeat(45), "ä".repeat(8));
        let short = shortened(&format!("{long}_owner_fkey"));
        assert!(short.as_str().len() <= MAX_IDENTIFIER_BYTES, "{short}");
        assert!(short.as_str().starts_with("aaaa"));
        assert_ne!(
            shortened(&format!("{long}_a_fkey")),
            shortened(&format!("{long}_b_fkey"))
        );
        assert_eq!(shortened("x").as_str(), "x");
        assert_eq!(shortened("").as_str().len(), 17);
    }

    #[test]
    fn limits() {
        assert!(SQLIdentifier::new("").is_err());
        assert!(SQLIdentifier::new("x".repeat(63)).is_ok());
        assert!(SQLIdentifier::new("x".repeat(64)).is_err());
        assert!(SQLIdentifier::new("a\0b").is_err());
    }
}
