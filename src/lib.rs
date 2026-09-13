/*
 * Copyright Lucas Käldström
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *      https://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

//! `cedar-sql` compiles Cedar policy evaluation into SQL.
//!
//! The Cedar schema becomes SQL DDL (one table per entity type, a tags table per
//! entity type with tags, and one entity-hierarchy table), and after typed
//! partial evaluation (TPE) the residual policies become a single query whose
//! CTEs fetch the referenced entity data, so the database computes every
//! policy's three-valued outcome (`true`, `false`, `NULL` for an error) and the
//! authorization decision. When the request's `principal` and/or `resource` are
//! unknown, the same query returns one row per candidate entity.
//!
//! See `README.md` for the design and `docs/plans/` for the implementation
//! phases. The pipeline is:
//!
//! 1. [`config`] / [`annotations`] / [`ddl`]: Cedar schema → [`config::DatabaseConfiguration`] → DDL.
//! 2. [`load`]: Cedar entities → rows (used by the tests and the differential tests).
//! 3. [`plan`] / [`compile`]: TPE residuals → query plan → SQL text.
//! 4. [`authorizer`]: the public entry points, computing the decision from the rows.
//! 5. [`backend`]: executing SQL on a database.
//!
//! **This crate is experimental and must not be used in production.**

#![warn(missing_docs)]
#![deny(unsafe_code)]

pub mod annotations;
pub mod authorizer;
pub mod backend;
pub mod compile;
pub mod config;
pub mod ddl;
pub mod dialect;
mod error;
pub mod ident;
pub mod load;
pub mod plan;
#[cfg(feature = "testing")]
pub mod testing;

pub use error::{Error, Result};
