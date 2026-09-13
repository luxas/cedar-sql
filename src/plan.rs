//! The query plan: which roots, joins and CTEs a set of residuals needs.
//!
//! Plan 3 adds `QueryPlan`, the roots (unknown request variables and
//! dereferenced entity literals), the attribute-path joins and the ancestor
//! CTEs, collected by a walk over the residuals.
