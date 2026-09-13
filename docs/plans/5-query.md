# Plan 5 — query: an unknown principal and/or resource, one row per candidate

## Goal

The README's second use: a request whose `principal` and/or `resource` is unknown returns, from the same
query, one row per candidate entity (every row of the type's table) or per pair, each with the decision, the
determining policies and the erroring policies of the concrete request it stands for.

## Design

- **Unknown roots** (`src/compile.rs`): `Compiler::ensure_unknown_roots` registers a root CTE for every
  request variable without an id, so the query enumerates the candidates even when no policy refers to them
  (a policy set that partial evaluation decided outright still yields one row per candidate, all with the
  folded decision). Unknown roots are `CROSS JOIN`ed, as before; a `Var` in a residual reads the root's id.
- **Rows** (`src/authorizer.rs`): `CompiledAuthorization::unknown_roots` lists the enumerated variables in
  order, and `row` decodes their ids into `EntityUid`s (the types come from the request) before the policy
  columns; `SqlAuthorizer::query` runs the query and returns `QueryRow { principal, resource, response }`
  per row; `is_authorized` rejects partial requests and `query` concrete ones. `partial_request` builds the
  `PartialRequest` from a concrete `Request` with the chosen ids dropped (`concrete_request` is the special
  case), and now rejects a request without a context, which `cedar-policy` treats as unknown, not empty.
- **Review fixes of branch 3**: `hasTag` on an entity type without tags is a guarded `FALSE` (the validator
  types it as `false`); JSON-stored contents are checked against their Cedar types when loading
  (`load::conforms`), since the compiled queries compare entity references inside sets and records by id
  under their static type; the connection sets `standard_conforming_strings = on`, which the literal
  quoting assumes.

## Files

`src/{compile,authorizer,load,backend/postgres}.rs`, `tests/{query_pg,hierarchy_pg,authorize_pg,load_pg}.rs`,
`tests/entities/kitchen_sink.json`, `docs/plans/5-query.md`.

## Verification

`cargo test --all-features` with `CEDAR_SQL_PG_URL` set: `tests/query_pg.rs` runs partial requests with the
principal, the resource and both unknown, and checks every row against `cedar_policy::Authorizer` on the
concrete candidate, the row set against the entities of the unknown types, and the allowed set against
`PolicySet::query_resource`; `tests/hierarchy_pg.rs` runs the recursive ancestors CTE over a table holding
only direct edges, with a cycle; `tests/authorize_pg.rs` gains the review's probes (quoted and over-long
literal ids, `like` over values with `%`, `_`, `\` and a newline, same-type elements on the right of `in`,
`x in x`, a join and an ancestors check on one path, `hasTag` on a tagless type). Then clippy and fmt.

## History

New; the fifth `cedar-sql` branch. The fixes for the review of branch 3 landed here, since branch 4 had
rewritten the compiler in the meantime.
