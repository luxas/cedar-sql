//! The SQL dialect differences.

use crate::config::SQLType;

/// What differs between databases in the SQL this crate emits.
pub trait Dialect {
    /// The dialect's name, for messages.
    fn name(&self) -> &'static str;
    /// The column type for `ty`.
    fn render_type(&self, ty: &SQLType) -> String;
    /// The column clause for a stored column computed from `expr`.
    fn generated_column(&self, expr: &str) -> String;
    /// A JSON literal from its serialized text (already single-quoted).
    fn json_literal(&self, quoted: &str) -> String;
    /// A 64-bit integer literal. `i64::MIN` needs care: as a bare literal
    /// Postgres parses it as the negation of a too-large number.
    fn bigint_literal(&self, n: i64) -> String;
}

/// Postgres 18.
#[derive(Debug, Clone, Copy, Default)]
pub struct Postgres;

impl Dialect for Postgres {
    fn name(&self) -> &'static str {
        "postgres"
    }

    fn render_type(&self, ty: &SQLType) -> String {
        match ty {
            SQLType::Text => "TEXT".into(),
            SQLType::BigInt => "BIGINT".into(),
            SQLType::Bool => "BOOLEAN".into(),
            SQLType::Jsonb => "JSONB".into(),
            SQLType::Set(element) => format!("{}[]", self.render_type(element)),
            SQLType::Custom(custom) => custom.clone(),
        }
    }

    fn generated_column(&self, expr: &str) -> String {
        format!("GENERATED ALWAYS AS ({expr}) STORED")
    }

    fn json_literal(&self, quoted: &str) -> String {
        format!("{quoted}::jsonb")
    }

    fn bigint_literal(&self, n: i64) -> String {
        format!("'{n}'::bigint")
    }
}
