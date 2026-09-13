//! Reading the `@sql_*` schema annotations.
//!
//! Plan 2 adds the entity-type annotations (`@sql_table`, `@sql_tags_table`,
//! `@sql_primary_key`, `@sql_custom_config`, `@sql_entity_id_column`) and the
//! attribute annotations (`@sql_column`, `@sql_unique`, `@sql_custom_attrs`),
//! read from the schema fragment since annotations do not survive into the
//! validator schema.
