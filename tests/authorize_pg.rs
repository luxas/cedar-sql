//! Concrete authorization through SQL against `cedar-policy`'s authorizer.
//!
//! Every policy here validates strictly (the compiler requires it), so
//! runtime errors come from entities that are missing from the store, from
//! dangling references, and from arithmetic overflow.

use std::collections::BTreeSet;
use std::str::FromStr;

use cedar_policy::{
    Authorizer, Context, Decision, Entities, EntityUid, PolicyId, PolicySet, Request,
};
use cedar_sql::Error;
use cedar_sql::authorizer::{Response, SqlAuthorizer};
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

    /// Authorizes through SQL and through `cedar-policy`; they must agree.
    fn check(&mut self, policies: &str, principal: &str, resource: &str) -> Response {
        let policies = PolicySet::from_str(policies).unwrap();
        let request = self.request(principal, resource);
        let sql = SqlAuthorizer::new(&self.schema, &self.config, &Postgres, &policies)
            .unwrap_or_else(|e| panic!("{policies}\n{e}"))
            .is_authorized(&mut self.db, &request, &self.entities)
            .unwrap_or_else(|e| panic!("{policies}\n{e}"));
        let expected = Authorizer::new().is_authorized(&request, &policies, &self.entities);
        let reason: BTreeSet<PolicyId> = expected.diagnostics().reason().cloned().collect();
        let errors: BTreeSet<PolicyId> = expected
            .diagnostics()
            .errors()
            .map(|e| match e {
                cedar_policy::AuthorizationError::PolicyEvaluationError(e) => e.policy_id().clone(),
            })
            .collect();
        assert_eq!(
            (sql.decision, &sql.reason, &sql.errors),
            (expected.decision(), &reason, &errors),
            "{policies}\n{request}"
        );
        sql
    }

    fn unsupported(&mut self, policies: &str, principal: &str, resource: &str) -> String {
        let policies = PolicySet::from_str(policies).unwrap();
        let request = self.request(principal, resource);
        match SqlAuthorizer::new(&self.schema, &self.config, &Postgres, &policies)
            .unwrap()
            .is_authorized(&mut self.db, &request, &self.entities)
        {
            Err(Error::Unsupported(what)) => what.to_owned(),
            other => panic!("expected an unsupported error, got {other:?}"),
        }
    }
}

const ALICE: &str = "User::\"alice\"";
const BOB: &str = "User::\"bob\"";
const CAROL: &str = "User::\"carol\"";
const NOBODY: &str = "User::\"nobody\"";
const D1: &str = "Doc::\"d1\"";
const GHOST_NAME: &str = "User::\"ghost\".name == \"x\"";

fn permit(condition: &str) -> String {
    format!("permit(principal, action, resource) when {{ {condition} }};")
}

fn ids(response: &Response) -> (Vec<String>, Vec<String>) {
    (
        response.reason.iter().map(ToString::to_string).collect(),
        response.errors.iter().map(ToString::to_string).collect(),
    )
}

impl Fixture {
    /// The single policy errors.
    fn errored(&mut self, policies: &str, principal: &str, resource: &str) {
        let r = self.check(policies, principal, resource);
        assert_eq!(
            ids(&r),
            (vec![], vec!["policy0".to_owned()]),
            "{policies} {principal}"
        );
    }

    /// No policy errors.
    fn clean(&mut self, policies: &str, principal: &str, resource: &str) -> Response {
        let r = self.check(policies, principal, resource);
        assert!(r.errors.is_empty(), "{policies} {principal}: {r:?}");
        r
    }
}

#[test]
fn attributes_and_errors() {
    let mut fx = Fixture::new(true);
    let p = permit("principal.name == \"Alice\"");
    assert_eq!(fx.check(&p, ALICE, D1).decision, Decision::Allow);
    assert_eq!(fx.check(&p, BOB, D1).decision, Decision::Deny);
    // A missing principal row errors on access and is false for `has`.
    fx.errored(&p, NOBODY, D1);
    fx.clean(
        &permit("principal has age && principal.age > 18"),
        NOBODY,
        D1,
    );
    // Optional attributes.
    assert_eq!(
        fx.check(
            &permit("principal has age && principal.age > 18"),
            ALICE,
            D1
        )
        .decision,
        Decision::Allow
    );
    fx.clean(&permit("principal has age && principal.age > 18"), BOB, D1);
    fx.clean(&permit("principal has age"), BOB, D1);
    // Chains: an existing friend, a dangling friend (`has` is false, an access errors).
    let friend =
        "principal has friend && principal.friend has name && principal.friend.name == \"Bob\"";
    assert_eq!(
        fx.check(&permit(friend), ALICE, D1).decision,
        Decision::Allow
    );
    fx.clean(&permit(friend), BOB, D1);
    fx.clean(
        &permit("principal has friend && principal.friend has name"),
        CAROL,
        D1,
    );
    fx.errored(
        &permit("principal has friend && principal.friend.name == \"x\""),
        CAROL,
        D1,
    );
    fx.clean(
        &permit("principal has friend && principal.friend has friend && principal.friend.friend has name"),
        ALICE,
        D1,
    );
    // Literal roots, existing and not.
    assert_eq!(
        fx.check(&permit("User::\"alice\".name == \"Alice\""), BOB, D1)
            .decision,
        Decision::Allow
    );
    fx.errored(&permit(GHOST_NAME), BOB, D1);
    // Several policies dereferencing the same literal share its CTE.
    let r = fx.check(
        &format!(
            "{}\n{}",
            permit("User::\"alice\".name == \"Alice\""),
            permit("User::\"alice\".admin")
        ),
        BOB,
        D1,
    );
    assert_eq!(
        ids(&r),
        (vec!["policy0".to_owned(), "policy1".to_owned()], vec![])
    );
    // The same attribute referenced twice, and a resource attribute.
    assert_eq!(
        fx.check(
            &permit("resource.owner == principal && resource.owner == User::\"alice\""),
            ALICE,
            D1
        )
        .decision,
        Decision::Allow
    );
}

#[test]
fn records_like_is_arithmetic() {
    let mut fx = Fixture::new(true);
    assert_eq!(
        fx.check(&permit("principal.profile.city == \"Zürich\""), ALICE, D1)
            .decision,
        Decision::Allow
    );
    assert_eq!(
        fx.check(&permit("principal.profile has boss"), ALICE, D1)
            .decision,
        Decision::Allow
    );
    assert_eq!(
        fx.check(&permit("principal.profile has boss"), BOB, D1)
            .decision,
        Decision::Deny
    );
    assert_eq!(
        fx.check(
            &permit("principal.profile has boss && principal.profile.boss == User::\"bob\""),
            ALICE,
            D1
        )
        .decision,
        Decision::Allow
    );
    fx.errored(&permit("principal.profile.city == \"x\""), NOBODY, D1);
    assert_eq!(
        fx.check(&permit("principal.name like \"Al*\""), ALICE, D1)
            .decision,
        Decision::Allow
    );
    assert_eq!(
        fx.check(&permit("principal.name like \"A_\""), ALICE, D1)
            .decision,
        Decision::Deny
    );
    assert_eq!(
        fx.check(&permit("principal.name like \"%\""), ALICE, D1)
            .decision,
        Decision::Deny
    );
    assert_eq!(
        fx.check(&permit("principal.name like \"*\\\\*\""), ALICE, D1)
            .decision,
        Decision::Deny
    );
    assert_eq!(
        fx.check(&permit("principal.name like \"*e\""), ALICE, D1)
            .decision,
        Decision::Allow
    );
    fx.errored(&permit("principal.name like \"*\""), NOBODY, D1);
    assert_eq!(
        fx.check(
            &permit("principal has friend && principal.friend is User"),
            ALICE,
            D1
        )
        .decision,
        Decision::Allow
    );
    assert_eq!(
        fx.check(
            &permit("principal has age && principal.age * 2 == 60"),
            ALICE,
            D1
        )
        .decision,
        Decision::Allow
    );
    assert_eq!(
        fx.check(
            &permit("principal has age && -(principal.age) == -30"),
            ALICE,
            D1
        )
        .decision,
        Decision::Allow
    );
    fx.errored(
        &permit("principal has age && principal.age + 9223372036854775807 > 0"),
        ALICE,
        D1,
    );
    fx.errored(
        &permit("principal has age && principal.age - 9223372036854775807 - 32 < 0"),
        ALICE,
        D1,
    );
    // 30 - MAX - 31 is exactly i64::MIN, whose negation overflows.
    fx.errored(
        &permit("principal has age && -(principal.age - 9223372036854775807 - 31) > 0"),
        ALICE,
        D1,
    );
    fx.clean(
        &permit("principal has age && -(principal.age - 9223372036854775807 - 30) > 0"),
        ALICE,
        D1,
    );
    fx.errored(
        &permit("principal has age && principal.age * 9223372036854775807 > 0"),
        ALICE,
        D1,
    );
    fx.clean(
        &permit("principal has age && principal.age * -307445734561825860 < 0"),
        ALICE,
        D1,
    );
    assert_eq!(
        fx.check(
            &permit("principal has age && principal.age < 31 && principal.age <= 30"),
            ALICE,
            D1
        )
        .decision,
        Decision::Allow
    );
    fx.clean(
        &permit("if principal has age then principal.age > 18 else false"),
        BOB,
        D1,
    );
    // `false && error` is false; `true && error` is an error; the untaken branch of
    // an `if` is never evaluated, but an erroring condition errors.
    fx.clean(
        &permit(&format!("principal.name == \"Bob\" && {GHOST_NAME}")),
        ALICE,
        D1,
    );
    fx.errored(
        &permit(&format!("principal.name == \"Bob\" && {GHOST_NAME}")),
        BOB,
        D1,
    );
    fx.clean(
        &permit(&format!("principal.name == \"Bob\" || {GHOST_NAME}")),
        BOB,
        D1,
    );
    fx.errored(
        &permit(&format!("principal.name == \"Alice\" || {GHOST_NAME}")),
        BOB,
        D1,
    );
    fx.clean(
        &permit(&format!(
            "if principal.name == \"Bob\" then true else {GHOST_NAME}"
        )),
        BOB,
        D1,
    );
    fx.errored(
        &permit(&format!("if {GHOST_NAME} then true else false")),
        BOB,
        D1,
    );
    fx.errored(&permit(&format!("!({GHOST_NAME})")), BOB, D1);
    // Different entity types are never equal; a missing operand still errors.
    fx.clean(
        &permit("principal has friend && principal.friend == resource.owner"),
        ALICE,
        D1,
    );
    assert_eq!(
        fx.check(
            &permit("principal has friend && principal.friend == resource.owner"),
            ALICE,
            D1
        )
        .decision,
        Decision::Deny
    );
    fx.errored(&permit("resource.owner == Doc::\"nope\".owner"), ALICE, D1);
    fx.clean(&permit("context.ip == \"1.2.3.4\""), ALICE, D1);
    assert_eq!(
        fx.unsupported(&permit("principal.groups == [\"a\"]"), ALICE, D1),
        "equality of sets or records"
    );
}

#[test]
fn hierarchy_and_tags() {
    for closed in [true, false] {
        let mut fx = Fixture::new(closed);
        let allow = |fx: &mut Fixture, p: &str, principal: &str| {
            assert_eq!(
                fx.check(&permit(p), principal, D1).decision,
                Decision::Allow,
                "{p} {principal} closed={closed}"
            );
        };
        let deny = |fx: &mut Fixture, p: &str, principal: &str| {
            let r = fx.clean(&permit(p), principal, D1);
            assert_eq!(
                r.decision,
                Decision::Deny,
                "{p} {principal} closed={closed}"
            );
        };
        allow(&mut fx, "principal in Group::\"admins\"", ALICE);
        deny(&mut fx, "principal in Group::\"admins\"", BOB);
        allow(&mut fx, "resource in Group::\"admins\"", BOB);
        allow(
            &mut fx,
            "principal in [Group::\"x\", Group::\"admins\"]",
            ALICE,
        );
        deny(&mut fx, "principal in [Group::\"x\", Group::\"y\"]", ALICE);
        deny(&mut fx, "principal in Group::\"admins\"", NOBODY);
        allow(&mut fx, "Doc::\"d1\".owner in Group::\"admins\"", BOB);
        deny(&mut fx, "Doc::\"d1\".owner in Group::\"nope\"", BOB);
        // Carol's friend does not exist, Bob has none, Alice's is not a member.
        deny(
            &mut fx,
            "principal has friend && principal.friend in Group::\"admins\"",
            CAROL,
        );
        deny(
            &mut fx,
            "principal has friend && principal.friend in Group::\"admins\"",
            BOB,
        );
        deny(
            &mut fx,
            "principal has friend && principal.friend in Group::\"admins\"",
            ALICE,
        );
        deny(
            &mut fx,
            "principal has friend && principal.friend in principal",
            CAROL,
        );
        allow(&mut fx, "resource.owner in principal", ALICE);
        deny(&mut fx, "resource.owner in principal", BOB);
        allow(&mut fx, "principal in resource.owner", ALICE);
        deny(&mut fx, "resource in resource.owner", ALICE);
        // A missing entity on the left of `in` is false, an errored operand errors.
        deny(&mut fx, "User::\"ghost\" in principal", ALICE);
        fx.errored(&permit("Doc::\"nope\".owner in principal"), ALICE, D1);
        fx.errored(&permit("principal in Doc::\"nope\".owner"), ALICE, D1);
        allow(
            &mut fx,
            "principal.hasTag(\"k\") && principal.getTag(\"k\") == \"v\"",
            ALICE,
        );
        deny(
            &mut fx,
            "principal.hasTag(\"k\") && principal.getTag(\"k\") == \"v\"",
            BOB,
        );
        deny(
            &mut fx,
            "principal.hasTag(\"k\") && principal.getTag(\"k\") == \"v\"",
            NOBODY,
        );
        allow(
            &mut fx,
            "principal.hasTag(\"it's\") && principal.getTag(\"it's\") == \"q'\"",
            ALICE,
        );
        allow(
            &mut fx,
            "resource.hasTag(\"reviewer\") && resource.getTag(\"reviewer\") == User::\"bob\"",
            ALICE,
        );
        deny(&mut fx, "resource.hasTag(principal.name)", ALICE);
        fx.errored(&permit("resource.hasTag(User::\"ghost\".name)"), ALICE, D1);
        assert_eq!(
            fx.unsupported(
                &permit(
                    "resource.hasTag(\"reviewer\") && resource.getTag(\"reviewer\").name == \"Bob\""
                ),
                ALICE,
                D1
            ),
            "attribute access on a computed entity value"
        );
    }
}

#[test]
fn decisions() {
    let mut fx = Fixture::new(true);
    let both = format!(
        "{}\nforbid(principal, action, resource) when {{ principal.name == \"Alice\" }};",
        permit("true")
    );
    let r = fx.check(&both, ALICE, D1);
    assert_eq!(r.decision, Decision::Deny);
    assert_eq!(ids(&r), (vec!["policy1".to_owned()], vec![]));
    let r = fx.check(&both, BOB, D1);
    assert_eq!(r.decision, Decision::Allow);
    assert_eq!(ids(&r), (vec!["policy0".to_owned()], vec![]));
    // An erroring policy is reported and ignored for the decision.
    let two = format!(
        "{}\n{}",
        permit(GHOST_NAME),
        permit("principal.name == \"Bob\"")
    );
    let r = fx.check(&two, BOB, D1);
    assert_eq!(r.decision, Decision::Allow);
    assert_eq!(
        ids(&r),
        (vec!["policy1".to_owned()], vec!["policy0".to_owned()])
    );
    let r = fx.check(&two, ALICE, D1);
    assert_eq!(r.decision, Decision::Deny);
    assert_eq!(ids(&r), (vec![], vec!["policy0".to_owned()]));
    // A forbid that errors does not deny.
    let forbid_errs = format!(
        "{}\nforbid(principal, action, resource) when {{ {GHOST_NAME} }};",
        permit("true")
    );
    let r = fx.check(&forbid_errs, ALICE, D1);
    assert_eq!(r.decision, Decision::Allow);
    assert_eq!(
        ids(&r),
        (vec!["policy0".to_owned()], vec!["policy1".to_owned()])
    );
    // Nothing residual: no query at all.
    fx.check(
        "permit(principal == User::\"alice\", action, resource);",
        ALICE,
        D1,
    );
    fx.check(
        "permit(principal == User::\"alice\", action, resource);",
        BOB,
        D1,
    );
    // A residual policy next to folded ones.
    let mixed = format!(
        "permit(principal == User::\"alice\", action, resource);\n{}",
        permit("principal.name == \"Bob\"")
    );
    let r = fx.check(&mixed, ALICE, D1);
    assert_eq!(ids(&r), (vec!["policy0".to_owned()], vec![]));
    let r = fx.check(&mixed, BOB, D1);
    assert_eq!(ids(&r), (vec!["policy1".to_owned()], vec![]));
    // Policies that do not validate are rejected up front.
    let policies = PolicySet::from_str(&permit("principal.nope == 1")).unwrap();
    assert!(matches!(
        SqlAuthorizer::new(&fx.schema, &fx.config, &Postgres, &policies),
        Err(Error::Validation(_))
    ));
}
