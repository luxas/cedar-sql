# Follow-ups for branch 6 — corpus

## PR description

The integration corpus runs through the crate from `cedar-sql-spec` (`docs/plans/cedar-sql-6-corpus.md`):
of 7497 corpus tests, 4304 (34413 requests) are inside the envelope and all agree with the expected
results; the 24 handwritten tests all pass. This branch records that in the README, compiles `hasTag` on
an action type as `false` (found by the corpus), and applies the review of branches 4 and 5.

## Review findings of branches 4 and 5 (fixed here)

- Generated join aliases joined attribute path segments with `.`, which an attribute name may contain.
- The differential tests matched loader errors on the substring `NUL`; the crate now has `Error::Nul`.
- Missing tests: a same-type principal and resource both unknown, a candidate type without rows, an enum
  type, a dotted attribute name.

## Suggested follow-ups

- Extension types (Plan 7): 580 corpus tests and 864 requests wait on them.
- The enumerated candidates of an enum entity type are its table rows, not its declared values (the same
  choice `PolicySet::query_resource` makes); worth stating in the README when Plan 7 touches enums.
