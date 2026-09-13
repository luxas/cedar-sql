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
    // The deferred foreign keys hold, except carol's dangling friend.
    let violation = db
        .execute_batch("SET CONSTRAINTS ALL IMMEDIATE")
        .expect_err("carol's friend does not exist");
    assert!(
        violation.to_string().contains("User_friend_fkey"),
        "{violation}"
    );
    db.rollback().unwrap();
    db.begin().unwrap();
    let mut without_fks = config.clone();
    without_fks.emit_foreign_keys = false;
    for statement in create_tables(&without_fks, &Postgres).unwrap() {
        db.execute_batch(&statement).unwrap();
    }
    for statement in &load.statements {
        db.execute_batch(statement).unwrap();
    }

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
            vec![
                text("carol"),
                text("User"),
                text("Carol"),
                SqlValue::Null,
                SqlValue::Bool(false),
                SqlValue::Json(json!(["x"])),
                SqlValue::Json(json!({"r": {"city": "Oslo", "pets": [7]}})),
                text("ghost"),
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
fn annotated_round_trip() {
    let src = std::fs::read_to_string("tests/schemas/readme_annotated.cedarschema").unwrap();
    let (config, schema) = DatabaseConfiguration::from_cedarschema_str(&src).unwrap();
    let json = std::fs::read_to_string("tests/entities/readme_annotated.json").unwrap();
    let entities = Entities::from_json_str(&json, Some(&schema)).unwrap();
    let load = entities_to_sql(&entities, &schema, &config, &Postgres).unwrap();
    let mut db = SharedPostgres::get().unwrap().connect().unwrap();
    db.begin().unwrap();
    for statement in create_tables(&config, &Postgres).unwrap() {
        db.execute_batch(&statement).unwrap();
    }
    for statement in &load.statements {
        db.execute_batch(statement)
            .unwrap_or_else(|e| panic!("{statement}\n{e}"));
    }
    db.execute_batch("SET CONSTRAINTS ALL IMMEDIATE").unwrap();
    let users = db
        .query("SELECT \"__entity_id3\", \"__entity_type2\", \"user_id\", \"custom_config\", \"__entity_id2\" FROM \"App::User\"")
        .unwrap();
    assert_eq!(
        users,
        vec![vec![
            text("u1"),
            text("App::User"),
            text("u1"),
            SqlValue::Json(json!({"r": {"a": true, "b": "x"}})),
            text("other"),
        ]]
    );
    let tags = db
        .query("SELECT \"entity_id\", \"tag\", \"value\" FROM \"App::User_tags\"")
        .unwrap();
    assert_eq!(tags, vec![vec![text("u1"), text("role"), text("t1")]]);
    let usertags = db
        .query("SELECT \"__entity_id3\", \"custom_pk\", \"custom_eid\", \"categories\", \"enabled\" FROM \"usertags\" ORDER BY 2")
        .unwrap();
    assert_eq!(
        usertags,
        vec![
            vec![
                text("t1"),
                SqlValue::Long(1),
                text("t1"),
                SqlValue::Json(json!(["a", "b"])),
                SqlValue::Bool(false)
            ],
            vec![
                text("t2"),
                SqlValue::Long(2),
                text("t2"),
                SqlValue::Null,
                SqlValue::Bool(true)
            ],
        ]
    );
    let hierarchy_entities = db
        .query("SELECT \"__entity_id3\", \"__entity_id\", \"__entity_type\" FROM \"cedar_entity_hierarchy\"")
        .unwrap();
    assert_eq!(
        hierarchy_entities,
        vec![vec![text("h"), SqlValue::Long(5), text("s")]]
    );
    // The id column exposed as an attribute must agree with the entity id.
    let bad = Entities::from_json_str(
        r#"[{"uid": {"type": "App::User", "id": "u9"}, "attrs": {"user_id": "u1", "config": {"a": true, "b": "x"}, "__entity_id2": "o"}, "parents": []}]"#,
        None,
    )
    .unwrap();
    let e = entities_to_sql(&bad, &schema, &config, &Postgres).expect_err("an error");
    assert!(e.to_string().contains("must equal the entity id"), "{e}");
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
    // Entity references must be of the referenced type; strings and references
    // do not mix.
    assert!(
        case(r#"[{"uid": {"type": "Doc", "id": "d"}, "attrs": {"owner": {"__entity": {"type": "Group", "id": "g"}}}, "parents": []}]"#)
            .contains("referencing User")
    );
    assert!(
        case(
            r#"[{"uid": {"type": "Doc", "id": "d"}, "attrs": {"owner": "alice"}, "parents": []}]"#
        )
        .contains("referencing User")
    );
    assert!(
        case(r#"[{"uid": {"type": "Group", "id": "g"}, "attrs": {}, "parents": [], "tags": {}}, {"uid": {"type": "User", "id": "u"}, "attrs": {"name": {"__entity": {"type": "User", "id": "x"}}, "admin": true, "groups": [], "profile": {"city": "", "pets": []}, "friends": []}, "parents": []}]"#)
            .contains("cannot be stored in a column of type Text")
    );
    assert!(
        case(r#"[{"uid": {"type": "Doc", "id": "d"}, "attrs": {"owner": {"__entity": {"type": "User", "id": "u"}}}, "parents": [], "tags": {"t": {"__entity": {"type": "Group", "id": "g"}}}}]"#)
            .contains("referencing User")
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
