//! Partial requests: one row per candidate, against brute-force concrete
//! authorization of every candidate and against `PolicySet::query_resource`.

use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;

use cedar_policy::{
    Authorizer, Context, Decision, Entities, EntityUid, PolicyId, PolicySet, Request,
    ResourceQueryRequest, Validator,
};
use cedar_sql::authorizer::{QueryRow, Response, SqlAuthorizer, action_entities, partial_request};
use cedar_sql::backend::Backend;
use cedar_sql::backend::postgres::PgBackend;
use cedar_sql::config::DatabaseConfiguration;
use cedar_sql::ddl::create_tables;
use cedar_sql::dialect::Postgres;
use cedar_sql::load::entities_to_sql;
use cedar_sql::testing::SharedPostgres;

struct Fixture {
    schema: cedar_policy::Schema,
    config: DatabaseConfiguration,
    entities: Entities,
    db: PgBackend,
}

impl Fixture {
    fn new(closed: bool) -> Self {
        let src = std::fs::read_to_string("tests/schemas/kitchen_sink.cedarschema").unwrap();
        let (mut config, schema) = DatabaseConfiguration::from_cedarschema_str(&src).unwrap();
        config.emit_foreign_keys = false;
        config.hierarchy_closed = closed;
        let json = std::fs::read_to_string("tests/entities/kitchen_sink.json").unwrap();
        let entities = Entities::from_json_str(&json, Some(&schema)).unwrap();
        let mut db = SharedPostgres::get().unwrap().connect().unwrap();
        db.begin().unwrap();
        for statement in create_tables(&config, &Postgres).unwrap() {
            db.execute_batch(&statement).unwrap();
        }
        let load = entities_to_sql(&entities, &schema, &config, &Postgres).unwrap();
        for statement in &load.statements {
            db.execute_batch(statement).unwrap();
        }
        Self {
            schema,
            config,
            entities,
            db,
        }
    }

    fn request(&self, principal: &str, resource: &str) -> Request {
        Request::new(
            EntityUid::from_str(principal).unwrap(),
            EntityUid::from_str("Action::\"view\"").unwrap(),
            EntityUid::from_str(resource).unwrap(),
            Context::from_json_str(r#"{"ip": "1.2.3.4"}"#, None).unwrap(),
            Some(&self.schema),
        )
        .unwrap()
    }

    fn expected(&self, request: &Request, policies: &PolicySet) -> Response {
        let response = Authorizer::new().is_authorized(request, policies, &self.entities);
        Response {
            decision: response.decision(),
            reason: response.diagnostics().reason().cloned().collect(),
            errors: response
                .diagnostics()
                .errors()
                .map(|e| match e {
                    cedar_policy::AuthorizationError::PolicyEvaluationError(e) => {
                        e.policy_id().clone()
                    }
                })
                .collect(),
        }
    }

    /// Runs the partial query and checks every row against the concrete
    /// authorizer, and the row set against the entities of the unknown types.
    fn check(
        &mut self,
        policies: &str,
        principal: &str,
        resource: &str,
        unknown_principal: bool,
        unknown_resource: bool,
    ) -> Vec<QueryRow> {
        let policies = PolicySet::from_str(policies).unwrap();
        let request = self.request(principal, resource);
        let partial =
            partial_request(&request, unknown_principal, unknown_resource, &self.schema).unwrap();
        let actions = action_entities(&self.entities, &self.schema).unwrap();
        let rows = SqlAuthorizer::new(&self.schema, &self.config, &Postgres, &policies)
            .unwrap()
            .query(&mut self.db, &partial, &actions)
            .unwrap_or_else(|e| panic!("{policies}\n{e}"));
        let candidates = |unknown: bool, uid: &EntityUid| -> Vec<EntityUid> {
            if unknown {
                self.entities
                    .iter()
                    .map(|e| e.uid())
                    .filter(|u| u.type_name() == uid.type_name())
                    .collect()
            } else {
                vec![uid.clone()]
            }
        };
        let principals = candidates(unknown_principal, request.principal().unwrap());
        let resources = candidates(unknown_resource, request.resource().unwrap());
        let mut expected = BTreeMap::new();
        for p in &principals {
            for r in &resources {
                let concrete = Request::new(
                    p.clone(),
                    request.action().unwrap().clone(),
                    r.clone(),
                    request.context().unwrap().clone(),
                    None,
                )
                .unwrap();
                expected.insert(
                    (p.to_string(), r.to_string()),
                    self.expected(&concrete, &policies),
                );
            }
        }
        let actual: BTreeMap<(String, String), Response> = rows
            .iter()
            .map(|row| {
                let p = row
                    .principal
                    .clone()
                    .unwrap_or_else(|| request.principal().unwrap().clone());
                let r = row
                    .resource
                    .clone()
                    .unwrap_or_else(|| request.resource().unwrap().clone());
                ((p.to_string(), r.to_string()), row.response.clone())
            })
            .collect();
        assert_eq!(actual.len(), rows.len(), "duplicate rows for {policies}");
        assert_eq!(actual, expected, "{policies}");
        rows
    }
}

const ALICE: &str = "User::\"alice\"";
const D1: &str = "Doc::\"d1\"";

fn permit(condition: &str) -> String {
    format!("permit(principal, action, resource) when {{ {condition} }};")
}

fn allowed(rows: &[QueryRow]) -> BTreeSet<String> {
    rows.iter()
        .filter(|r| r.response.decision == Decision::Allow)
        .map(|r| {
            r.principal
                .clone()
                .or_else(|| r.resource.clone())
                .unwrap()
                .to_string()
        })
        .collect()
}

#[test]
fn unknown_principal() {
    for closed in [true, false] {
        let mut fx = Fixture::new(closed);
        let rows = fx.check(
            &permit("principal.name like \"A*\""),
            ALICE,
            D1,
            true,
            false,
        );
        assert_eq!(rows.len(), 4);
        assert_eq!(
            allowed(&rows),
            BTreeSet::from(["User::\"alice\"".to_owned()])
        );
        let rows = fx.check(
            &permit("principal in Group::\"admins\""),
            ALICE,
            D1,
            true,
            false,
        );
        assert_eq!(
            allowed(&rows),
            BTreeSet::from(["User::\"alice\"".to_owned()])
        );
        // Errors per row: carol's friend does not exist.
        let rows = fx.check(
            &permit("principal has friend && principal.friend.name == \"Bob\""),
            ALICE,
            D1,
            true,
            false,
        );
        let errors: BTreeSet<String> = rows
            .iter()
            .filter(|r| !r.response.errors.is_empty())
            .map(|r| r.principal.clone().unwrap().to_string())
            .collect();
        assert_eq!(errors, BTreeSet::from(["User::\"carol\"".to_owned()]));
        assert_eq!(
            allowed(&rows),
            BTreeSet::from(["User::\"alice\"".to_owned()])
        );
        // Sets, tags and computed entities under an unknown root.
        fx.check(
            &permit("principal.groups.contains(\"a\")"),
            ALICE,
            D1,
            true,
            false,
        );
        fx.check(
            &permit("principal.hasTag(\"k\") && principal.getTag(\"k\") == \"v\""),
            ALICE,
            D1,
            true,
            false,
        );
        fx.check(
            &permit("principal in resource.owner || resource.owner in principal"),
            ALICE,
            D1,
            true,
            false,
        );
        fx.check(
            &permit("principal has friend && principal.friend in Group::\"admins\""),
            ALICE,
            D1,
            true,
            false,
        );
    }
}

#[test]
fn unknown_resource_and_both() {
    let mut fx = Fixture::new(true);
    let rows = fx.check(
        &permit("resource in Group::\"admins\""),
        ALICE,
        D1,
        false,
        true,
    );
    assert_eq!(rows.len(), 1);
    // No residual at all: every candidate gets the folded decision.
    let rows = fx.check(
        "permit(principal, action, resource);",
        ALICE,
        D1,
        false,
        true,
    );
    assert_eq!(allowed(&rows).len(), 1);
    let rows = fx.check(
        "permit(principal == User::\"zzz\", action, resource);",
        ALICE,
        D1,
        false,
        true,
    );
    assert!(allowed(&rows).is_empty() && rows.len() == 1);
    // Both unknown: one row per pair.
    let rows = fx.check(
        &permit("principal.name == \"Alice\" && resource.owner == principal"),
        ALICE,
        D1,
        true,
        true,
    );
    assert_eq!(rows.len(), 4);
    let rows = fx.check(
        "permit(principal, action, resource);",
        ALICE,
        D1,
        true,
        true,
    );
    assert_eq!(rows.len(), 4);
    // A forbid over both.
    fx.check(
        &format!(
            "{}\nforbid(principal, action, resource) when {{ principal.name == \"Bob\" || resource.owner == principal }};",
            permit("true")
        ),
        ALICE,
        D1,
        true,
        true,
    );
}

#[test]
fn agrees_with_query_resource() {
    let mut fx = Fixture::new(true);
    let policies = permit("resource.owner == principal || resource in Group::\"admins\"");
    let rows = fx.check(&policies, ALICE, D1, false, true);
    let policy_set = PolicySet::from_str(&policies).unwrap();
    let validator = Validator::new(fx.schema.clone());
    let request = fx.request(ALICE, D1);
    let query = ResourceQueryRequest::new(
        request.principal().unwrap().clone(),
        request.action().unwrap().clone(),
        request.resource().unwrap().type_name().clone(),
        request.context().unwrap().clone(),
        validator.schema(),
    )
    .unwrap();
    let expected: BTreeSet<String> = policy_set
        .query_resource(&query, &fx.entities, validator.schema())
        .unwrap()
        .map(|u| u.to_string())
        .collect();
    assert_eq!(allowed(&rows), expected);
    let _ = PolicyId::new("x");
}
