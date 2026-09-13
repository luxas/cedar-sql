# Plan 1 — init: the crate skeleton, Postgres for tests, CI

## Goal

`cedar-sql` compiles Cedar evaluation to SQL (`README.md`). This branch turns the empty repository into a
buildable crate with the module layout the later phases fill in, provisions a Postgres for the test suites
and the differential tests, and sets up CI, so that every later branch lands as code plus tests.

## Design

- **Crate** (`Cargo.toml`, `src/lib.rs`): depends on `cedar-policy` and `cedar-policy-core` with `tpe`,
  patched to the `../cedar` checkout as `cedar-spec` does; modules `ident`, `config`, `annotations`, `ddl`,
  `dialect`, `load`, `plan`, `compile`, `authorizer`, `backend`, `testing`, `error` — the last four have code,
  the rest document what their phase adds.
- **Error** (`src/error.rs`): `Error::Unsupported(&'static str)` names a construct the compiler does not
  handle yet (the differential tests skip these), `Database`, `Decode`, `Provision`.
- **Backend** (`src/backend/`): `trait Backend { execute_batch, query, begin, rollback }` over a
  `SqlValue {Null, Bool, Long, Text, Json}` row model; `PgBackend` on the synchronous `postgres` client,
  decoding `bool`, `int8`, `int4`, `text`, `jsonb`.
- **Testing** (`src/testing.rs`, feature `testing`): `SharedPostgres::get()` provisions one server per process
  behind a `OnceLock`: the one at `CEDAR_SQL_PG_URL` when set, otherwise `postgresql_embedded` (Postgres 18,
  temporary data directory, install directory overridable with `CEDAR_SQL_PG_INSTALL_DIR`, stopped through an
  `atexit` hook since a `static` is never dropped). Tests run `BEGIN … ROLLBACK`, so the database stays empty.
- **CI** (`.github/workflows/ci.yml`): a `postgres:18` service, `cargo fmt/build/clippy/test` with
  `-D warnings`, the `cedar-woodpecker` fork checked out next to the crate.
- **README**: a "Status and phases" section listing the phases and the two decided deviations.

## Files

`Cargo.toml`, `src/lib.rs`, `src/error.rs`, `src/backend/{mod,postgres}.rs`, `src/testing.rs`, the module
stubs, `tests/smoke_pg.rs`, `.github/workflows/ci.yml`, `README.md`, `docs/plans/1-init.md`.

## Verification

`cargo build --all-features`, `cargo clippy --all-targets --all-features -- -D warnings`,
`cargo fmt --all --check`, `cargo test --all-features` (with `CEDAR_SQL_PG_URL` set, or the embedded server).

## History

New; the first of the `cedar-sql` branches planned on 2026-09-13.
