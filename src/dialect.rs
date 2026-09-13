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
    /// The statements defining the helper functions the compiled queries call
    /// (see [`CEDAR_EQ`]), and the statements dropping them.
    fn helper_functions(&self) -> (Vec<String>, Vec<String>);
}

/// The name of the helper function comparing two canonical JSON values as
/// Cedar values: arrays as sets (each element of one has an equal element in
/// the other), objects by keys and values, everything else by JSON equality.
/// `STRICT`, so a `NULL` argument (an error) yields `NULL`.
pub const CEDAR_EQ: &str = "cedar_eq";

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

    fn helper_functions(&self) -> (Vec<String>, Vec<String>) {
        let create = format!(
            r#"CREATE OR REPLACE FUNCTION {CEDAR_EQ}(a jsonb, b jsonb) RETURNS boolean
LANGUAGE plpgsql IMMUTABLE STRICT AS $cedar$
BEGIN
  IF jsonb_typeof(a) = 'array' AND jsonb_typeof(b) = 'array' THEN
    RETURN NOT EXISTS (
        SELECT 1 FROM jsonb_array_elements(a) AS x
        WHERE NOT EXISTS (SELECT 1 FROM jsonb_array_elements(b) AS y WHERE {CEDAR_EQ}(x.value, y.value)))
      AND NOT EXISTS (
        SELECT 1 FROM jsonb_array_elements(b) AS y
        WHERE NOT EXISTS (SELECT 1 FROM jsonb_array_elements(a) AS x WHERE {CEDAR_EQ}(x.value, y.value)));
  ELSIF jsonb_typeof(a) = 'object' AND jsonb_typeof(b) = 'object' THEN
    RETURN (SELECT coalesce(array_agg(k ORDER BY k), '{{}}') FROM jsonb_object_keys(a) AS k)
         = (SELECT coalesce(array_agg(k ORDER BY k), '{{}}') FROM jsonb_object_keys(b) AS k)
      AND NOT EXISTS (
        SELECT 1 FROM jsonb_each(a) AS e WHERE NOT {CEDAR_EQ}(e.value, b -> e.key));
  ELSE
    RETURN a = b;
  END IF;
END
$cedar$"#
        );
        (
            vec![create],
            vec![format!("DROP FUNCTION IF EXISTS {CEDAR_EQ}(jsonb, jsonb)")],
        )
    }
}
