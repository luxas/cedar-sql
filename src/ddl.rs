//! Rendering a [`crate::config::DatabaseConfiguration`] as DDL.
//!
//! Plan 2 adds `create_tables` and `drop_tables`: the entity tables, the tags
//! tables, the hierarchy table, and the foreign keys as trailing `ALTER TABLE`
//! statements so table order does not matter.
