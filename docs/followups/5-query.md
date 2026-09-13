# Follow-ups for branch 5 — query (review)

## PR description

Partial requests: an unknown principal and/or resource yields one row per candidate, each with the decision,
determining and erroring policies of the concrete request it stands for (`docs/plans/5-query.md`), plus the
fixes of the branch 3 review.

## Review findings

No semantic bug: the row decoding, the unknown roots, `partial_request`, the `conforms` check and the zero-row
case all matched `cedar-policy` and `PolicySet::query_resource`. Findings, fixed in branch 6: the test suite
lacked a same-type principal and resource both unknown, a candidate type without rows, an enum type and
attribute names with dots; the crate signals NUL strings with a message the harness matched by substring.
Design note: the candidates of an enum entity type are its table rows, not its declared values, as for
`PolicySet::query_resource`.

## Suggested follow-ups

- Partial context (out of scope per the README) is the remaining unknown a request could carry.
