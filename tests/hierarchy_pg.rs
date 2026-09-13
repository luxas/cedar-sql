//! The recursive ancestors CTE over a hierarchy table holding only direct
//! edges — with a cycle — and the closed table holding the closure.

use std::str::FromStr;

use cedar_policy::{Authorizer, Context, Decision, Entities, EntityUid, PolicySet, Request};
use cedar_sql::authorizer::SqlAuthorizer;
use cedar_sql::backend::Backend;
use cedar_sql::config::DatabaseConfiguration;
use cedar_sql::ddl::create_tables;
use cedar_sql::dialect::Postgres;
use cedar_sql::testing::SharedPostgres;

const SCHEMA: &str = r#"
    entity G in [G];
    entity U in [G] = { boss?: U };
    action a appliesTo { principal: [U], resource: [G] };
"#;

/// Direct edges: u1 -> g1 -> g2 -> g3, g3 -> g1 (a cycle), u2 -> g3, u1.boss = u2.
const EDGES: &[(&str, &str, &str, &str)] = &[
    ("U", "u1", "G", "g1"),
    ("G", "g1", "G", "g2"),
    ("G", "g2", "G", "g3"),
    ("G", "g3", "G", "g1"),
    ("U", "u2", "G", "g3"),
];

/// The same data as Cedar entities, with the closure computed by Cedar.
const ENTITIES: &str = r#"[
    {"uid": {"type": "G", "id": "g1"}, "attrs": {}, "parents": [{"type": "G", "id": "g2"}]},
    {"uid": {"type": "G", "id": "g2"}, "attrs": {}, "parents": [{"type": "G", "id": "g3"}]},
    {"uid": {"type": "G", "id": "g3"}, "attrs": {}, "parents": [{"type": "G", "id": "g1"}]},
    {"uid": {"type": "U", "id": "u1"}, "attrs": {"boss": {"__entity": {"type": "U", "id": "u2"}}}, "parents": [{"type": "G", "id": "g1"}]},
    {"uid": {"type": "U", "id": "u2"}, "attrs": {}, "parents": [{"type": "G", "id": "g3"}]},
    {"uid": {"type": "Action", "id": "a"}, "attrs": {}, "parents": []}
]"#;

#[test]
fn direct_edges_with_a_cycle() {
    let (mut config, schema) = DatabaseConfiguration::from_cedarschema_str(SCHEMA).unwrap();
    config.emit_foreign_keys = false;
    config.hierarchy_closed = false;
    // Cedar rejects cycles when computing the closure, so the oracle uses a
    // hierarchy without the closing edge; the SQL side gets the cycle.
    let entities = Entities::from_json_str(
        &ENTITIES.replace(
            r#"[{"type": "G", "id": "g1"}]},
    {"uid": {"type": "U", "id": "u1"}"#,
            r#"[]},
    {"uid": {"type": "U", "id": "u1"}"#,
        ),
        Some(&schema),
    )
    .unwrap();
    let mut db = SharedPostgres::get().unwrap().connect().unwrap();
    db.begin().unwrap();
    for statement in create_tables(&config, &Postgres).unwrap() {
        db.execute_batch(&statement).unwrap();
    }
    for id in ["g1", "g2", "g3"] {
        db.execute_batch(&format!(
            "INSERT INTO \"G\" (\"__entity_id\") VALUES ('{id}')"
        ))
        .unwrap();
    }
    db.execute_batch(
        "INSERT INTO \"U\" (\"__entity_id\", \"boss\") VALUES ('u1', 'u2'), ('u2', NULL)",
    )
    .unwrap();
    for (dt, di, at, ai) in EDGES {
        db.execute_batch(&format!(
            "INSERT INTO \"cedar_entity_hierarchy\" VALUES ('{dt}', '{di}', '{at}', '{ai}')"
        ))
        .unwrap();
    }
    // (condition, principal, resource, expected, whether Cedar's cycle-free
    // closure gives the same answer)
    let cases = [
        ("principal in G::\"g1\"", "u1", "g1", Decision::Allow, true),
        ("principal in G::\"g3\"", "u1", "g1", Decision::Allow, true),
        ("principal in G::\"g1\"", "u2", "g1", Decision::Allow, false),
        ("principal in G::\"nope\"", "u1", "g1", Decision::Deny, true),
        ("resource in G::\"g1\"", "u1", "g2", Decision::Allow, false),
        ("resource in resource", "u1", "g2", Decision::Allow, true),
        (
            "principal has boss && principal.boss in G::\"g2\"",
            "u1",
            "g1",
            Decision::Allow,
            false,
        ),
        (
            "principal has boss && principal.boss in G::\"g2\"",
            "u2",
            "g1",
            Decision::Deny,
            true,
        ),
        (
            "principal in [G::\"x\", G::\"g2\"]",
            "u1",
            "g1",
            Decision::Allow,
            true,
        ),
        ("U::\"u2\" in G::\"g2\"", "u1", "g1", Decision::Allow, false),
        (
            "U::\"nobody\" in G::\"g2\"",
            "u1",
            "g1",
            Decision::Deny,
            true,
        ),
    ];
    for (condition, principal, resource, expected, cedar_agrees) in cases {
        let policies = PolicySet::from_str(&format!(
            "permit(principal, action, resource) when {{ {condition} }};"
        ))
        .unwrap();
        let request = Request::new(
            EntityUid::from_str(&format!("U::\"{principal}\"")).unwrap(),
            EntityUid::from_str("Action::\"a\"").unwrap(),
            EntityUid::from_str(&format!("G::\"{resource}\"")).unwrap(),
            Context::empty(),
            Some(&schema),
        )
        .unwrap();
        let response = SqlAuthorizer::new(&schema, &config, &Postgres, &policies)
            .unwrap()
            .is_authorized(&mut db, &request, &entities)
            .unwrap_or_else(|e| panic!("{condition}: {e}"));
        assert_eq!(
            response.decision, expected,
            "{condition} {principal} {resource}"
        );
        assert!(response.errors.is_empty(), "{condition}");
        if cedar_agrees {
            let cedar = Authorizer::new().is_authorized(&request, &policies, &entities);
            assert_eq!(cedar.decision(), expected, "cedar {condition} {principal}");
        }
    }
    db.rollback().unwrap();
}
