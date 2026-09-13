# Follow-ups for branch 4 — values (review)

## PR description

Sets, records, computed entities, `in` over sets and shared sub-expressions (`docs/plans/4-values.md`); after
it, the differential test in `cedar-sql-spec` reports no benign skip on generated inputs.

## Review findings

The review (about ninety probe conditions in both hierarchy modes, plus partial-request probes) found no
semantic bug in the generated SQL: `cedar_eq`, the JSON conversions, the `NULL` guards of set and record
literals, the computed-entity subqueries and the `LATERAL` sharing all matched `cedar-policy`. Its one code
finding — join aliases joining attribute path segments with `.`, so that `principal["b.c"]` and
`principal.b.c` collide with a loud SQL error — is fixed in branch 6.

## Suggested follow-ups

- Measure the planner's behaviour on the `LATERAL` chain for long policies (Plan 9's benchmarks).
