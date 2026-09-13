//! The public entry points: authorizing a request against the database.
//!
//! [`SqlAuthorizer::is_authorized`] runs typed partial evaluation with the
//! request (all of it known) and the action entities as the only known
//! entities, so that every policy either folds to `true`, `false` or an error,
//! or leaves a residual that references entity data; the residuals become one
//! query (see [`crate::compile`]) whose row yields each residual policy's
//! three-valued outcome. The decision, the determining policies and the
//! erroring policies are then computed as `cedar-policy`'s authorizer does:
//! `Allow` iff some permit is satisfied and no forbid is, the determining
//! policies are the satisfied forbids if any else the satisfied permits, and
//! the erroring policies are reported but do not affect the decision.

use std::collections::BTreeSet;

use cedar_policy::{
    Decision, Entities, EntityId, EntityTypeName, EntityUid, PolicyId, PolicySet, Request, Schema,
    ValidationMode, Validator,
};
use cedar_policy_core::ast::{Context, Effect, Entity, EntityUIDEntry};
use cedar_policy_core::tpe;
use cedar_policy_core::tpe::entities::{PartialEntities, PartialEntity};
use cedar_policy_core::tpe::request::{PartialEntityUID, PartialRequest};
use ref_cast::RefCast;

use crate::backend::{Backend, Row, SqlValue};
use crate::compile::{Compiler, RootKey};
use crate::config::DatabaseConfiguration;
use crate::dialect::Dialect;
use crate::{Error, Result};

/// An authorization response.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Response {
    /// The decision.
    pub decision: Decision,
    /// The determining policies.
    pub reason: BTreeSet<PolicyId>,
    /// The policies whose evaluation errored.
    pub errors: BTreeSet<PolicyId>,
}

/// One row of a partial-request query: a candidate for each unknown
/// variable, and the response for that candidate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueryRow {
    /// The principal, when it was unknown.
    pub principal: Option<EntityUid>,
    /// The resource, when it was unknown.
    pub resource: Option<EntityUid>,
    /// The response for this candidate.
    pub response: Response,
}

/// The outcome of partial evaluation and compilation for one request.
#[derive(Clone, Debug)]
pub struct CompiledAuthorization {
    /// The query, when some policies are residual or a request variable is
    /// unknown; `None` when partial evaluation decided every policy of a
    /// concrete request.
    pub sql: Option<String>,
    /// The residual policies, in the order of the query's policy columns.
    pub residuals: Vec<(PolicyId, Effect)>,
    /// The unknown request variables whose ids the query selects first, in
    /// order (`principal` before `resource`).
    pub unknown_roots: Vec<RootKey>,
    principal_type: EntityTypeName,
    resource_type: EntityTypeName,
    true_permits: BTreeSet<PolicyId>,
    true_forbids: BTreeSet<PolicyId>,
    errors: BTreeSet<PolicyId>,
}

impl CompiledAuthorization {
    /// The row's candidates and response.
    pub fn row(&self, row: &Row) -> Result<QueryRow> {
        let mut principal = None;
        let mut resource = None;
        for (root, value) in self.unknown_roots.iter().zip(row) {
            let SqlValue::Text(id) = value else {
                return Err(Error::Query(format!("a root id column holds {value:?}")));
            };
            let id = EntityId::new(id);
            match root {
                RootKey::Principal => {
                    principal = Some(EntityUid::from_type_name_and_id(
                        self.principal_type.clone(),
                        id,
                    ));
                }
                RootKey::Resource => {
                    resource = Some(EntityUid::from_type_name_and_id(
                        self.resource_type.clone(),
                        id,
                    ));
                }
                RootKey::Literal(_) => unreachable!("only variables are unknown roots"),
            }
        }
        Ok(QueryRow {
            principal,
            resource,
            response: self.response(Some(row))?,
        })
    }

    /// The response for one result row (`None` when there is no query).
    pub fn response(&self, row: Option<&Row>) -> Result<Response> {
        let mut permits = self.true_permits.clone();
        let mut forbids = self.true_forbids.clone();
        let mut errors = self.errors.clone();
        if let Some(row) = row {
            let roots = self.unknown_roots.len();
            if row.len() != roots + self.residuals.len() {
                return Err(Error::Query(format!(
                    "expected {} columns, got {}",
                    roots + self.residuals.len(),
                    row.len()
                )));
            }
            for ((id, effect), value) in self.residuals.iter().zip(&row[roots..]) {
                match value {
                    SqlValue::Bool(true) => {
                        match effect {
                            Effect::Permit => permits.insert(id.clone()),
                            Effect::Forbid => forbids.insert(id.clone()),
                        };
                    }
                    SqlValue::Bool(false) => {}
                    SqlValue::Null => {
                        errors.insert(id.clone());
                    }
                    other => {
                        return Err(Error::Query(format!(
                            "policy {id} yielded {other:?}, not a boolean"
                        )));
                    }
                }
            }
        } else if !self.residuals.is_empty() {
            return Err(Error::Query(
                "a row is needed for the residual policies".into(),
            ));
        }
        let decision = if !forbids.is_empty() || permits.is_empty() {
            Decision::Deny
        } else {
            Decision::Allow
        };
        let reason = if forbids.is_empty() { permits } else { forbids };
        Ok(Response {
            decision,
            reason,
            errors,
        })
    }
}

/// Authorizes requests for one schema, database configuration and policy set.
pub struct SqlAuthorizer<'a> {
    schema: &'a Schema,
    config: &'a DatabaseConfiguration,
    dialect: &'a dyn Dialect,
    policies: PolicySet,
}

impl<'a> SqlAuthorizer<'a> {
    /// Validates `policies` strictly against `schema`, which compilation
    /// relies on (every residual is typed).
    pub fn new(
        schema: &'a Schema,
        config: &'a DatabaseConfiguration,
        dialect: &'a dyn Dialect,
        policies: &PolicySet,
    ) -> Result<Self> {
        let result = Validator::new(schema.clone()).validate(policies, ValidationMode::Strict);
        if !result.validation_passed() {
            return Err(Error::Validation(
                result
                    .validation_errors()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join("; "),
            ));
        }
        Ok(Self {
            schema,
            config,
            dialect,
            policies: policies.clone(),
        })
    }

    /// Partially evaluates the policies for `request` over `entities` (the
    /// action entities, typically) and compiles the residuals.
    pub fn compile(
        &self,
        request: &PartialRequest,
        entities: &PartialEntities,
    ) -> Result<CompiledAuthorization> {
        let schema = self.schema.as_ref();
        let response = tpe::is_authorized(self.policies.as_ref(), request, entities, schema)
            .map_err(|e| Error::Tpe(e.to_string()))?;
        let ids = |policies: Box<dyn Iterator<Item = &tpe::response::ResidualPolicy> + '_>| {
            policies
                .map(|p| PolicyId::new(p.get_policy_id()))
                .collect::<BTreeSet<_>>()
        };
        let true_permits = ids(Box::new(response.true_permits()));
        let true_forbids = ids(Box::new(response.true_forbids()));
        let mut errors = ids(Box::new(response.error_permits()));
        errors.extend(ids(Box::new(response.error_forbids())));
        let mut residuals: Vec<_> = response
            .residual_permits()
            .chain(response.residual_forbids())
            .collect();
        residuals.sort_by_key(|p| p.get_policy_id().to_string());
        let principal_type = EntityTypeName::ref_cast(request.principal_type()).clone();
        let resource_type = EntityTypeName::ref_cast(request.resource_type()).clone();
        let mut compiler = Compiler::new(self.config, schema, self.dialect, request);
        compiler.ensure_unknown_roots();
        if residuals.is_empty() && compiler.unknown_roots().is_empty() {
            return Ok(CompiledAuthorization {
                sql: None,
                residuals: Vec::new(),
                unknown_roots: Vec::new(),
                principal_type,
                resource_type,
                true_permits,
                true_forbids,
                errors,
            });
        }
        let mut columns = Vec::new();
        let mut order = Vec::new();
        for (i, policy) in residuals.iter().enumerate() {
            let condition = compiler.condition(&policy.get_residual())?;
            columns.push((format!("p{i}"), condition));
            order.push((PolicyId::new(policy.get_policy_id()), policy.get_effect()));
        }
        let sql = compiler.render(&columns)?;
        Ok(CompiledAuthorization {
            sql: Some(sql),
            residuals: order,
            unknown_roots: compiler.unknown_roots(),
            principal_type,
            resource_type,
            true_permits,
            true_forbids,
            errors,
        })
    }

    /// Authorizes a partial `request` — an unknown principal and/or
    /// resource — over the database behind `db` and the action entities
    /// `entities`: one row per candidate entity of the unknown type(s) (every
    /// row of its table), or per pair when both are unknown.
    pub fn query(
        &self,
        db: &mut dyn Backend,
        request: &PartialRequest,
        entities: &PartialEntities,
    ) -> Result<Vec<QueryRow>> {
        let compiled = self.compile(request, entities)?;
        if compiled.unknown_roots.is_empty() {
            return Err(Error::Request(
                "the request has no unknown principal or resource; use is_authorized".into(),
            ));
        }
        let sql = compiled
            .sql
            .as_ref()
            .expect("an unknown root always renders a query");
        db.query(sql)?.iter().map(|row| compiled.row(row)).collect()
    }

    /// Authorizes a concrete `request`, with the entity data in the database
    /// behind `db` and the action entities in `actions`.
    pub fn is_authorized(
        &self,
        db: &mut dyn Backend,
        request: &Request,
        actions: &Entities,
    ) -> Result<Response> {
        let request = concrete_request(request, self.schema)?;
        let entities = action_entities(actions, self.schema)?;
        let compiled = self.compile(&request, &entities)?;
        if !compiled.unknown_roots.is_empty() {
            return Err(Error::Request(
                "the request has an unknown principal or resource; use query".into(),
            ));
        }
        let Some(sql) = &compiled.sql else {
            return compiled.response(None);
        };
        let rows = db.query(sql)?;
        let [row] = rows.as_slice() else {
            return Err(Error::Query(format!(
                "expected exactly one row, got {}",
                rows.len()
            )));
        };
        compiled.response(Some(row))
    }
}

/// The partial request with everything known.
pub fn concrete_request(request: &Request, schema: &Schema) -> Result<PartialRequest> {
    partial_request(request, false, false, schema)
}

/// The partial request of `request` with the principal and/or the resource
/// id dropped (their types stay known), for [`SqlAuthorizer::query`].
pub fn partial_request(
    request: &Request,
    unknown_principal: bool,
    unknown_resource: bool,
    schema: &Schema,
) -> Result<PartialRequest> {
    let core = request.as_ref();
    let known = |entry: &EntityUIDEntry, what: &str, unknown: bool| match entry {
        EntityUIDEntry::Known { euid, .. } => Ok(PartialEntityUID {
            ty: euid.entity_type().clone(),
            eid: if unknown {
                None
            } else {
                Some(euid.eid().clone())
            },
        }),
        EntityUIDEntry::Unknown { .. } => Err(Error::Request(format!("the {what} is unknown"))),
    };
    let principal = known(core.principal(), "principal", unknown_principal)?;
    let resource = known(core.resource(), "resource", unknown_resource)?;
    let action = match core.action() {
        EntityUIDEntry::Known { euid, .. } => euid.as_ref().clone(),
        EntityUIDEntry::Unknown { .. } => {
            return Err(Error::Request("the action is unknown".into()));
        }
    };
    let context = match core.context() {
        Some(Context::Value(values)) => Some(values.clone()),
        Some(Context::RestrictedResidual(_)) => {
            return Err(Error::Unsupported("a partially unknown context"));
        }
        // A request without a context is one with an *unknown* context, which
        // `cedar-policy` evaluates as an unknown; it is not the empty record.
        None => return Err(Error::Request("the context is unknown".into())),
    };
    PartialRequest::new(principal, action, resource, context, schema.as_ref())
        .map_err(|e| Error::Request(e.to_string()))
}

/// The action entities of `entities` as known partial entities.
pub fn action_entities(entities: &Entities, schema: &Schema) -> Result<PartialEntities> {
    let actions = entities
        .as_ref()
        .iter()
        .filter(|e| e.uid().entity_type().is_action())
        .map(|e: &Entity| PartialEntity::try_from(e.clone()).map_err(|e| Error::Tpe(e.to_string())))
        .collect::<Result<Vec<_>>>()?;
    PartialEntities::from_entities(actions.into_iter(), schema.as_ref())
        .map_err(|e| Error::Tpe(e.to_string()))
}
