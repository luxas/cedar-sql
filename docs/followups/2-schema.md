# Follow-ups for branch 2 — schema (review fixes)

## PR description

Idea 1 of the README: a Cedar schema with its `@sql_*` annotations becomes a `DatabaseConfiguration`, renders
as Postgres DDL, and Cedar entities load as rows (`docs/plans/2-schema.md`). Tests run the README's schemas'
DDL on Postgres and read loaded rows back.

## What this branch contains

`SQLIdentifier` and literal quoting; the configuration model and its builder from the validator schema and
the annotations; DDL with deferred foreign keys; the loader with the canonical JSON encoding of compound values.

## Review findings (fixed here)

- Constraint-name shortening looped forever on long names with multibyte characters; now `ident::shortened`
  cuts at a char boundary and appends a hash (shared with the query compiler's generated names).
- With `@sql_primary_key` naming another column, the `__entity_id` column was not unique, so foreign keys to it
  failed; it is now `UNIQUE` whenever it is not the primary key.
- The loader stored entity references of the wrong type, and strings in reference columns, silently; `literal`
  now checks the referenced entity type (`ForeignKey::entity_type`) for attributes and tag values.
- Attribute annotations behind a qualified or cross-namespace common type name were dropped; `resolve_record`
  resolves `Ns::Type` in the named namespace.
- `@sql_entity_id_column` naming an undefined column now adds a `TEXT NOT NULL UNIQUE` column, as the README says.
- Conflicting `@sql_entity_id_column`/`@sql_primary_key` and `@sql_custom_config` values are errors; record keys
  are NUL-checked; generated interface columns are `NOT NULL`.

## Divergences from the README

Recorded in the README's "Status and phases": canonical JSONB for sets and records, the closure in the
hierarchy table, the primary key accepted as the entity id column, deferred foreign keys.

## Suggested follow-ups

- Arrays as a fast path for flat scalar sets (Plan 8 or later).
- A `CHECK`-style validation query for the canonical form of loaded JSON.
