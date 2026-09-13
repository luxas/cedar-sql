# Plan 3 — is-authorized: concrete authorization through one query

## Goal

Idea 2 of the README for a concrete request: typed partial evaluation with the request known and only the
action entities in the store, the residual policies compiled into one query over the tables of Plan 2, and
the decision with its determining and erroring policies rebuilt as `cedar-policy`'s authorizer computes them.
This branch covers the core operators; sets, records beyond `has`/`get`, tags on computed entities and the
remaining constructs are `Unsupported` until Plan 4, which the differential test in `cedar-sql-spec` treats
as benign skips.

## Design

- **Residuals** (`src/authorizer.rs`): `concrete_request` turns a `Request` into a `PartialRequest` with both
  ids and the context known; `action_entities` turns the action entities into `PartialEntities`;
  `cedar_policy_core::tpe::is_authorized` then folds everything that does not touch entity data. `SqlAuthorizer::
  new` validates the policies strictly (every residual node is typed, which the compiler relies on).
- **The encoding** (`src/compile.rs`): `NULL` is a Cedar error. `&&`, `||` and `if` are `CASE` forms that keep
  short-circuiting (`false && error` is `false`, `error && false` an error, an untaken branch is not evaluated);
  `+ - *` compute in `numeric` and range-check so no bigint overflow reaches Postgres (which would abort the
  statement); unary `-` guards `i64::MIN`; `==` on the same representation is `=`, on different entity types or
  representations it is `false` unless an operand is `NULL`; `<`/`<=` on integers; `like` translates the pattern
  to `LIKE … ESCAPE '\'` (`*` to `%`, `%`/`_`/`\` escaped); `is` is a constant from the static type; `hasTag`
  is a guarded `EXISTS` and `getTag` a scalar subquery on the tags table (no row is an error); `in` is
  `(same type AND same id) OR EXISTS` on the hierarchy table when it holds the closure, else on a per-anchor
  recursive ancestors CTE (`UNION`, so cycles terminate); `in` on a literal set is the disjunction. Record
  attributes read the canonical JSON (`-> 'r' -> 'k'`, cast per static type; `has` is `?`).
- **Roots and CTEs**: a `Compiled` value carries its representation and, for entities fetched from a row, an
  `Anchor` (root plus attribute path). Roots are the unknown request variables and the entity literals that are
  dereferenced; a root's CTE selects the id column as `"$id"`, one `LEFT JOIN` per entity-typed attribute path
  under it (joined on the target's entity id column, so at most one row each), and one `"$v:<path>"` column per
  referenced attribute path. `getAttr` is that column (`NULL` for a missing row or an absent attribute, both
  Cedar errors); `hasAttr` is `IS NOT NULL` guarded by the parent's value, so a missing entity is `false` and an
  errored parent an error; an attribute the schema does not declare is `false`. Generated names longer than 63
  bytes are shortened with a hash (`ident::shortened`), since Postgres would truncate them silently.
- **The query**: `WITH [RECURSIVE] <roots>, <ancestors> SELECT <unknown ids>, p0 … pN FROM (SELECT 1 AS base)
  CROSS JOIN <unknown roots> LEFT JOIN <literal roots> ON TRUE`, so a concrete request yields exactly one row.
- **Decision** (`CompiledAuthorization::response`): satisfied permits and forbids are the ones TPE found `true`
  plus the residual columns that are `TRUE`; `errors` are TPE's plus the `NULL` columns; `Allow` iff some permit
  and no forbid; the reason is the satisfied forbids if any, else the satisfied permits.

## Files

`src/{compile,authorizer,error}.rs`, `tests/authorize_pg.rs`, `tests/entities/kitchen_sink.json` (a dangling
reference), `tests/load_pg.rs`, `docs/plans/3-is-authorized.md`.

## Verification

`cargo test --all-features` with `CEDAR_SQL_PG_URL` set: `tests/authorize_pg.rs` authorizes every case through
SQL and through `cedar_policy::Authorizer` and requires the same decision, reason set and error set — attribute
chains over existing, absent and dangling references, literal roots, records, `like`, `is`, arithmetic at the
overflow boundary, short-circuiting with an erroring operand on each side, `in` on both hierarchy modes, tags,
and the decision rules (forbid overriding, erroring permits and forbids, folded policies next to residual ones).
Then `cargo clippy --all-targets --all-features -- -D warnings` and `cargo fmt --all --check`.

## History

New; the third `cedar-sql` branch. The differential fuzz target is `cedar-sql-spec` branch `cedar-sql-3-is-authorized`.
