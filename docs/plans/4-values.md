# Plan 4 — values: sets, records, computed entities, and shared sub-expressions

## Goal

Every construct the type-directed generators produce (without extension types) compiles: sets and records
end to end, entity values that are not row-anchored chains, `in` over sets, and a query whose size stays
linear in the policy. After this branch the differential test in `cedar-sql-spec` asserts that no benign
skip remains.

## Design

- **Set and record equality** (`src/dialect.rs`, `src/ddl.rs`): `create_tables` installs the PL/pgSQL helper
  `cedar_eq(a jsonb, b jsonb)` (`STRICT`, so an error operand stays `NULL`): arrays compare as sets (each
  element of one has an equal element in the other, recursively), objects by key set and values, anything
  else by JSON equality. No element ordering is relied on anywhere, so the loader's canonical order is only
  for determinism.
- **Set operations** (`src/compile.rs`): `==` on two JSON values is `cedar_eq`; `contains` is an `EXISTS`
  over `jsonb_array_elements` with `cedar_eq` against the element (native values converted with `to_jsonb`
  after an explicit cast, entities wrapped as `{"e": {"t", "i"}}`); `containsAll` is a double `NOT EXISTS`,
  `containsAny` a join; `isEmpty` is `jsonb_array_length = 0`; every one is guarded by a `NULL` check of its
  inputs. `Set` and `Record` nodes with computed children build `jsonb_build_array`/`jsonb_build_object`
  under a guard, since a `NULL` child would silently become JSON `null`.
- **Computed entities**: an entity value that is not a root-anchored chain (a tag value, the result of an
  `if`, a record field) is dereferenced by a scalar subquery on its type's table: `getAttr` is
  `(SELECT t.col FROM table t WHERE t.id = v)` (no row is `NULL`, an error), `hasAttr` is `COALESCE((SELECT
  t.col IS NOT NULL …), FALSE)` under a guard on `v` (a missing entity is `false`). With a closed hierarchy
  table `in` works on any entity value; with the recursive CTE only anchored values are supported.
- **`in` over sets**: the right side may be any JSON set of entities, literal or computed: `EXISTS` over its
  elements, each checked for identity or ancestry (dynamic ancestor type and id from the element).
- **Sharing**: `Compiler::share` binds an operand used more than once — the guard of `&&`, `||` and `if`,
  an arithmetic result, `in`/tag/set operands — to a `CROSS JOIN LATERAL (SELECT … AS "v") AS "nK"` node, so
  each sub-expression appears once and a chain of `n` conjunctions renders in `O(n)`, not `O(2^n)`.
- **Names** (`src/config.rs`): `DatabaseConfiguration::from_schema_shortening_names` shortens over-long
  default table, tags table and column names with `ident::shortened` instead of demanding an annotation, for
  schemas nobody annotates (generated ones); `from_schema` keeps the README's rule.

## Files

`src/{compile,config,dialect,ddl,error}.rs`, `tests/{authorize_pg,config,ddl_golden}.rs`, `tests/golden/*.sql`,
`docs/plans/4-values.md`.

## Verification

`cargo test --all-features` with `CEDAR_SQL_PG_URL` set: `tests/authorize_pg.rs` gains set and record
equality, `contains*`, `isEmpty`, set and record literals with computed elements, `if`-computed entities,
tag values dereferenced and used on the left of `in`, computed sets on the right of `in`, and a check that
doubling a conjunction chain at most doubles the query text; the goldens gain the helper function. Then
`cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --all --check`, and the
`sql-is-authorized-drt` smoke test in `cedar-sql-spec` with no benign skips.

## History

New; the fourth `cedar-sql` branch.
