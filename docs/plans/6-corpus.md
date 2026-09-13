# Plan 6 — corpus: results of the integration tests

## Goal

No code: this branch records that the `cedar-integration-tests` corpus runs through the crate
(`cedar-sql-spec` branch `cedar-sql-6-corpus`) and what its envelope is, in the README.

## Design

- The README's "Testing" section states the envelope (strictly validating policies, no extension types, no
  NUL strings) and the results, and points to the differential tests.

## Files

`README.md`, `docs/plans/6-corpus.md`.

## Verification

See `cedar-sql-spec`'s `docs/plans/cedar-sql-6-corpus.md`.

## History

New; the sixth `cedar-sql` branch.
