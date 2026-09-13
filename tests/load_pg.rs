//! Loading entities into the generated tables and reading them back.

use std::collections::HashSet;

use cedar_policy::{Entities, Entity, EntityId, EntityUid};
use cedar_sql::backend::{Backend, SqlValue};
use cedar_sql::config::DatabaseConfiguration;
use cedar_sql::ddl::create_tables;
use cedar_sql::dialect::Postgres;
use cedar_sql::load::entities_to_sql;
use cedar_sql::testing::SharedPostgres;
use serde_json::json;

fn text(s: &str) -> SqlValue {
    SqlValue::Text(s.into())
}

#[test]
fn round_trip() {
    let src = std::fs::read_to_string("tests/schemas/kitchen_sink.cedarschema").unwrap();
    let (config, schema) = DatabaseConfiguration::from_cedarschema_str(&src).unwrap();
    let json = std::fs::read_to_string("tests/entities/kitchen_sink.json").unwrap();
    let entities = Entities::from_json_str(&json, Some(&schema)).unwrap();
    let load = entities_to_sql(&entities, &schema, &config, &Postgres).unwrap();
    assert_eq!(load.actions.len(), 1);
    assert_eq!(load.actions[0].uid().to_string(), "Action::\"view\"");

    let mut db = SharedPostgres::get().unwrap().connect().unwrap();
    db.begin().unwrap();
    for statement in create_tables(&config, &Postgres).unwrap() {
        db.execute_batch(&statement).unwrap();
    }
    for statement in &load.statements {
        db.execute_batch(statement)
            .unwrap_or_else(|e| panic!("{statement}\n{e}"));
    }
    // The deferred foreign keys hold.
    db.execute_batch("SET CONSTRAINTS ALL IMMEDIATE").unwrap();

    let users = db
        .query(
            "SELECT \"__entity_id\", \"__entity_type\", \"name\", \"age\", \"admin\", \"groups\", \"profile\", \"friend\", \"friends\" FROM \"User\" ORDER BY 1",
        )
        .unwrap();
    let bob = json!({"e": {"t": "User", "i": "bob"}});
    assert_eq!(
        users,
        vec![
            vec![
                text("alice"),
                text("User"),
                text("Alice"),
                SqlValue::Long(30),
                SqlValue::Bool(true),
                SqlValue::Json(json!(["a", "b"])),
                SqlValue::Json(json!({"r": {"boss": bob, "city": "Zürich", "pets": [1, 2, 3]}})),
                text("bob"),
                SqlValue::Json(json!([bob])),
            ],
            vec![
                text("bob"),
                text("User"),
                text("Bob"),
                SqlValue::Null,
                SqlValue::Bool(false),
                SqlValue::Json(json!([])),
                SqlValue::Json(json!({"r": {"city": "", "pets": []}})),
                SqlValue::Null,
                SqlValue::Json(json!([])),
            ],
        ]
    );
    let hierarchy = db
        .query("SELECT * FROM \"cedar_entity_hierarchy\" ORDER BY 1, 2")
        .unwrap();
    assert_eq!(
        hierarchy,
        vec![
            vec![text("Doc"), text("d1"), text("Group"), text("admins")],
            vec![text("User"), text("alice"), text("Group"), text("admins")],
        ]
    );
    let tags = db
        .query("SELECT \"entity_id\", \"tag\", \"value\" FROM \"User_tags\" ORDER BY 2")
        .unwrap();
    assert_eq!(
        tags,
        vec![
            vec![text("alice"), text("it's"), text("q'")],
            vec![text("alice"), text("k"), text("v")],
        ]
    );
    let doc_tags = db
        .query("SELECT \"entity_id\", \"tag\", \"value\" FROM \"Doc_tags\"")
        .unwrap();
    assert_eq!(
        doc_tags,
        vec![vec![text("d1"), text("reviewer"), text("bob")]]
    );
    db.rollback().unwrap();
}

#[test]
fn rejects_bad_entities() {
    let src = std::fs::read_to_string("tests/schemas/kitchen_sink.cedarschema").unwrap();
    let (config, schema) = DatabaseConfiguration::from_cedarschema_str(&src).unwrap();
    let case = |entities: &str| {
        let entities = Entities::from_json_str(entities, None).unwrap();
        entities_to_sql(&entities, &schema, &config, &Postgres)
            .expect_err("an error")
            .to_string()
    };
    assert!(
        case(r#"[{"uid": {"type": "Group", "id": "g"}, "attrs": {"extra": 1}, "parents": []}]"#)
            .contains("not declared in the schema")
    );
    assert!(
        case(r#"[{"uid": {"type": "User", "id": "u"}, "attrs": {"name": "x"}, "parents": []}]"#)
            .contains("required attribute")
    );
    assert!(
        case(r#"[{"uid": {"type": "Group", "id": "g"}, "attrs": {}, "parents": [], "tags": {"t": 1}}]"#)
            .contains("declares no tags")
    );
    assert!(
        case(r#"[{"uid": {"type": "Nope", "id": "g"}, "attrs": {}, "parents": []}]"#)
            .contains("no table stores")
    );
    // A NUL character, which Postgres cannot store, is rejected up front.
    let nul = Entity::new_no_attrs(
        EntityUid::from_type_name_and_id("Group".parse().unwrap(), EntityId::new("a\0b")),
        HashSet::new(),
    );
    let entities = Entities::from_entities([nul], None).unwrap();
    let e = entities_to_sql(&entities, &schema, &config, &Postgres).expect_err("an error");
    assert!(e.to_string().contains("NUL"), "{e}");
}
