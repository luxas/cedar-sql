# Follow-ups for branch 3 — is-authorized (review)

## PR description

Idea 2 for a concrete request: partial evaluation with the request and the action entities known, the
residuals compiled into one query with the three-valued encoding, the decision rebuilt as `cedar-policy`
does (`docs/plans/3-is-authorized.md`). Tested against `cedar_policy::Authorizer` case by case, and by the
`sql-is-authorized-drt` target in `cedar-sql-spec`.

## Review findings

The review (against the committed branch, with some forty probe policies) found no semantic divergence in
the SQL encoding. Its findings, fixed in branch 5 since branch 4 rewrote the compiler in between:

- A request without a context is one with an unknown context in `cedar-policy`; it was treated as empty.
- Entity references inside JSON-stored sets and records are compared by id under their static type; the
  loader now checks stored contents against the schema (`load::conforms`).
- The quoting assumed `standard_conforming_strings`; connections now set it.
- `hasTag` on an entity type without tags (which the validator types as `false`) was unsupported.
- The query text grew exponentially with nested `&&`/`||`; branch 4 binds shared operands to `LATERAL`
  nodes.
- On the `cedar-sql-spec` side: loader errors other than NUL strings and `Request`/`Tpe` errors after
  validation are bugs, not skips; `enable_extensions: false` made typed generation abort inputs at the
  extension-function arms; the recursive hierarchy mode and `like` were not fuzzed.

## Suggested follow-ups

- Probes worth keeping as tests were added to `tests/authorize_pg.rs` and `tests/hierarchy_pg.rs` (branch 5).
