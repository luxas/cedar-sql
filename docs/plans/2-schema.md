# Plan 2 — schema: the database configuration, DDL and the entity loader

## Goal

Idea 1 of the README end to end: a Cedar schema, with its `@sql_*` annotations, becomes a
`DatabaseConfiguration` (tables, columns, primary keys, tags tables, the hierarchy table and the
`__entity_id`/`__entity_type` interface columns), the configuration renders as Postgres DDL, and Cedar
entities load as rows of those tables — so that later phases have data to query.

## Design

- **Identifiers** (`src/ident.rs`): `SQLIdentifier` (non-empty, at most 63 bytes, no NUL; `Display` is the
  double-quoted form) and `quoted_literal` (single quotes doubled; NUL rejected, since Postgres `text` cannot
  hold it).
- **Model** (`src/config.rs`): `SQLType {Text, BigInt, Bool, Jsonb, Set, Custom}`, `ColumnConfiguration`
  (type, nullable, unique, custom attributes, foreign key, generated expression), `TableConfiguration`
  (entity type, primary keys, columns in definition order, the tags table, the entity id column, the
  attribute-to-column map), `DatabaseConfiguration` (tables in entity-type order, the hierarchy table and the
  interface column names, `emit_foreign_keys`, `hierarchy_closed`). `SQLType::for_cedar_type` maps `Bool`,
  `Long`, `String`, a single entity type (a `Text` id with a foreign key) and, per the decision recorded in the
  README, `Set` and `Record` to `Jsonb`; extension, union, unspecified and never types are `Unsupported`, as are
  entity types with additional attributes.
- **The builder**: entity types sorted by name; the table is `@sql_table` or the fully-qualified type name;
  one column per attribute (`@sql_column`, `@sql_unique`, `@sql_custom_attrs`), nullable iff optional; the
  custom columns, primary keys and entity id column of `@sql_custom_config` (JSON, `entity_type` must be
  `null`); the tags table is `@sql_tags_table` or `<table>_tags`. Conflicts are errors naming the annotation
  that resolves them: two attributes on one column, a custom column on an attribute column, two entity types on
  one table, a tags table on an entity table. The interface columns and the hierarchy table take the default
  name unless any table's column (or any table) uses it, then the first free numeric suffix from 2 — the
  README's `__entity_id2`/`cedar_entity_hierarchy2` rule. `@sql_entity_id_column` must name a non-nullable
  text column that is unique or the primary key; the interface id column is then generated from it, otherwise
  it is the physical primary key. The type column is always generated from the type name.
- **DDL** (`src/ddl.rs`, `src/dialect.rs`): `create_tables` renders every entity table, tags table
  (`entity_id, tag, value`, key `(entity_id, tag)`) and the hierarchy table, then the foreign keys as
  `ALTER TABLE … DEFERRABLE INITIALLY DEFERRED` so table order and reference cycles do not matter and rows load
  in any order; constraint names over 63 bytes are shortened with a hash. `drop_tables` is the inverse. The
  `Dialect` trait (Postgres only for now) owns the type names, the generated-column clause and the literal forms.
- **Loader** (`src/load.rs`): `entities_to_sql` renders one `INSERT` per entity, per ancestor (the hierarchy
  table gets the transitive closure Cedar entities already carry) and per tag; action entities are returned
  separately for partial evaluation. `canonical_json` is the JSON encoding of compound values: sets as arrays
  (deduplicated by Cedar's own set semantics), records as `{"r": {…}}`, entity references as
  `{"e": {"t": type, "i": id}}`; equality of these is defined order-insensitively by later phases, so no
  element ordering is relied on. Undeclared attributes, missing required attributes, tags on tagless types,
  unknown types and NUL characters are errors; residual (unknown) attribute values are `Unsupported`.

## Files

`Cargo.toml`, `src/{ident,config,annotations,ddl,dialect,load,error}.rs`, `tests/{config,ddl_golden,load_pg}.rs`,
`tests/schemas/*.cedarschema`, `tests/golden/*.sql`, `tests/entities/kitchen_sink.json`, `docs/plans/2-schema.md`.

## Verification

`cargo fmt --all --check`, `cargo clippy --all-targets --all-features -- -D warnings`,
`cargo test --all-features` with `CEDAR_SQL_PG_URL` set: the README's two schemas and a kitchen-sink schema
against golden DDL (`UPDATE_GOLDEN=1` regenerates) that also executes on Postgres, the configuration rules and
error cases, and a load round trip that reads the rows, hierarchy and tags back and checks the deferred
foreign keys with `SET CONSTRAINTS ALL IMMEDIATE`.

## History

New; the second `cedar-sql` branch.
