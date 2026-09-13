//! The schema-to-configuration mapping: names, conflicts and validation.

use cedar_sql::Error;
use cedar_sql::config::{DatabaseConfiguration, SQLType};

fn config(src: &str) -> Result<DatabaseConfiguration, Error> {
    DatabaseConfiguration::from_cedarschema_str(src).map(|(c, _)| c)
}

fn err(src: &str) -> String {
    match config(src) {
        Ok(c) => panic!("expected an error, got {c:#?}"),
        Err(e) => e.to_string(),
    }
}

#[test]
fn defaults() {
    let c = config(
        r#"
        namespace App {
            entity Group;
            entity User in [Group] = { name: String, boss?: User, tags: Set<Long>, r: { a: Bool } } tags String;
            action view appliesTo { principal: [User], resource: [Group] };
        }"#,
    )
    .unwrap();
    assert_eq!(c.entity_id_column.as_str(), "__entity_id");
    assert_eq!(c.entity_type_column.as_str(), "__entity_type");
    assert_eq!(c.entity_hierarchy_table.as_str(), "cedar_entity_hierarchy");
    let names: Vec<&str> = c.tables.keys().map(|k| k.as_str()).collect();
    assert_eq!(names, ["App::Group", "App::User"]);
    let user = &c.tables["App::User"];
    let columns: Vec<(&str, &SQLType, bool)> = user
        .columns
        .iter()
        .map(|(k, v)| (k.as_str(), &v.ty, v.nullable))
        .collect();
    assert_eq!(
        columns,
        [
            ("__entity_id", &SQLType::Text, false),
            ("__entity_type", &SQLType::Text, false),
            ("boss", &SQLType::Text, true),
            ("name", &SQLType::Text, false),
            ("r", &SQLType::Jsonb, false),
            ("tags", &SQLType::Jsonb, false),
        ]
    );
    assert_eq!(
        user.columns["boss"]
            .references
            .as_ref()
            .unwrap()
            .table
            .as_str(),
        "App::User"
    );
    assert_eq!(
        user.columns["boss"]
            .references
            .as_ref()
            .unwrap()
            .column
            .as_str(),
        "__entity_id"
    );
    assert_eq!(user.tags.as_ref().unwrap().table.as_str(), "App::User_tags");
    assert_eq!(user.tags.as_ref().unwrap().value.ty, SQLType::Text);
    assert!(user.columns["__entity_type"].generated.as_deref() == Some("'App::User'"));
    let ety = "App::User".parse().unwrap();
    assert_eq!(c.column_for(&ety, "name").unwrap().as_str(), "name");
    assert!(c.table_for(&"App::Action".parse().unwrap()).is_none());
}

#[test]
fn interface_suffixes() {
    let c = config(
        r#"
        entity cedar_entity_hierarchy = { __entity_id: Long, __entity_type: String };
        entity Other = { __entity_id2: Bool, __entity_type2: Bool, __entity_type3: Bool };
        action a appliesTo { principal: [Other], resource: [Other] };
        "#,
    )
    .unwrap();
    assert_eq!(c.entity_id_column.as_str(), "__entity_id3");
    assert_eq!(c.entity_type_column.as_str(), "__entity_type4");
    assert_eq!(c.entity_hierarchy_table.as_str(), "cedar_entity_hierarchy2");
}

#[test]
fn column_conflicts() {
    let e = err(r#"
        entity User = { @sql_column("name") fullName: String, @sql_column("name") firstName: String };
        action a appliesTo { principal: [User], resource: [User] };
        "#);
    assert!(e.contains("both map to column \"name\""), "{e}");
    let e = err(r#"
        @sql_custom_config("{\"columns\": {\"name\": {\"ty\": \"Text\"}}}")
        entity User = { name: String };
        action a appliesTo { principal: [User], resource: [User] };
        "#);
    assert!(e.contains("collides with an attribute column"), "{e}");
}

#[test]
fn table_conflicts() {
    let e = err(r#"
        @sql_table("t") entity A;
        @sql_table("t") entity B;
        action a appliesTo { principal: [A], resource: [B] };
        "#);
    assert!(e.contains("use @sql_table"), "{e}");
    let e = err(r#"
        entity foo tags String;
        entity foo_tags;
        action a appliesTo { principal: [foo], resource: [foo_tags] };
        "#);
    assert!(e.contains("use @sql_tags_table"), "{e}");
}

#[test]
fn entity_id_column_rules() {
    let e = err(r#"
        @sql_entity_id_column("id") entity User = { id: Long };
        action a appliesTo { principal: [User], resource: [User] };
        "#);
    assert!(e.contains("non-nullable text column"), "{e}");
    let e = err(r#"
        @sql_entity_id_column("id") entity User = { id: String };
        action a appliesTo { principal: [User], resource: [User] };
        "#);
    assert!(e.contains("unique or the primary key"), "{e}");
    let c = config(
        r#"
        @sql_entity_id_column("id") @sql_primary_key("id") entity User = { id: String };
        action a appliesTo { principal: [User], resource: [User] };
        "#,
    )
    .unwrap();
    let user = &c.tables["User"];
    assert_eq!(user.entity_id_column.as_str(), "id");
    assert_eq!(
        user.columns["__entity_id"].generated.as_deref(),
        Some("\"id\"")
    );
    assert_eq!(
        user.primary_keys
            .iter()
            .map(|k| k.as_str())
            .collect::<Vec<_>>(),
        ["id"]
    );
}

#[test]
fn primary_key_elsewhere_keeps_the_id_unique() {
    let c = config(
        r#"
        @sql_primary_key("id") entity User = { id: String };
        entity Doc = { owner: User };
        action a appliesTo { principal: [User], resource: [Doc] };
        "#,
    )
    .unwrap();
    let user = &c.tables["User"];
    assert!(user.columns["__entity_id"].unique);
    assert_eq!(
        user.primary_keys
            .iter()
            .map(|k| k.as_str())
            .collect::<Vec<_>>(),
        ["id"]
    );
    let owner = c.tables["Doc"].columns["owner"]
        .references
        .as_ref()
        .unwrap();
    assert_eq!(
        (owner.table.as_str(), owner.column.as_str()),
        ("User", "__entity_id")
    );
    assert_eq!(owner.entity_type.to_string(), "User");
    // The default table's id column is the primary key and not separately unique.
    assert!(!c.tables["Doc"].columns["__entity_id"].unique);
}

#[test]
fn undefined_entity_id_column_is_added() {
    let c = config(
        r#"
        @sql_entity_id_column("external_id") entity User = { name: String };
        action a appliesTo { principal: [User], resource: [User] };
        "#,
    )
    .unwrap();
    let user = &c.tables["User"];
    let added = &user.columns["external_id"];
    assert!(added.unique && !added.nullable && added.ty == SQLType::Text);
    assert_eq!(user.entity_id_column.as_str(), "external_id");
    assert_eq!(
        user.columns["__entity_id"].generated.as_deref(),
        Some("\"external_id\"")
    );
}

#[test]
fn annotation_and_custom_config_conflicts() {
    let e = err(r#"
        @sql_entity_id_column("a")
        @sql_custom_config("{\"entity_id_column\": \"b\", \"columns\": {\"a\": {\"ty\": \"Text\", \"unique\": true}, \"b\": {\"ty\": \"Text\", \"unique\": true}}}")
        entity User;
        action a appliesTo { principal: [User], resource: [User] };
        "#);
    assert!(e.contains("different entity id columns"), "{e}");
    let e = err(r#"
        @sql_primary_key("a")
        @sql_custom_config("{\"primary_keys\": [\"b\"], \"columns\": {\"a\": {\"ty\": \"Text\"}, \"b\": {\"ty\": \"Text\"}}}")
        entity User;
        action a appliesTo { principal: [User], resource: [User] };
        "#);
    assert!(e.contains("different primary keys"), "{e}");
}

#[test]
fn bad_annotations() {
    let e = err(r#"
        entity User = { @sql_unique("yes") id: String };
        action a appliesTo { principal: [User], resource: [User] };
        "#);
    assert!(
        e.contains("@sql_unique") && e.contains("attribute id of User"),
        "{e}"
    );
    let e = err(r#"
        @sql_custom_config("{\"entity_type\": \"User\"}") entity User;
        action a appliesTo { principal: [User], resource: [User] };
        "#);
    assert!(e.contains("@sql_custom_config"), "{e}");
    let e = err(r#"
        @sql_table("") entity User;
        action a appliesTo { principal: [User], resource: [User] };
        "#);
    assert!(e.contains("@sql_table") && e.contains("empty"), "{e}");
}

#[test]
fn unsupported() {
    let e = err(r#"
        entity User = { ip: ipaddr };
        action a appliesTo { principal: [User], resource: [User] };
        "#);
    assert_eq!(e, "unsupported: extension types");
    let e = err(r#"
        entity User = { a: Action };
        action a appliesTo { principal: [User], resource: [User] };
        "#);
    assert_eq!(e, "unsupported: attributes referencing action entity types");
}

#[test]
fn json_common_type_annotations() {
    let (c, _) = DatabaseConfiguration::from_json_str(
        r#"{
          "Lib": {"entityTypes": {}, "actions": {}, "commonTypes": {"Shape": {"type": "Record", "attributes": {"name": {"type": "String", "annotations": {"sql_column": "lib_name"}}}}}},
          "App": {
            "commonTypes": {"Shape": {"type": "Record", "attributes": {"name": {"type": "String", "annotations": {"sql_column": "n"}}}}},
            "entityTypes": {
              "Plain": {"shape": {"type": "Shape"}},
              "Qualified": {"shape": {"type": "App::Shape"}},
              "Cross": {"shape": {"type": "Lib::Shape"}}
            },
            "actions": {"a": {"appliesTo": {"principalTypes": ["Plain"], "resourceTypes": ["Qualified", "Cross"]}}}
          }
        }"#,
    )
    .unwrap();
    assert_eq!(
        c.tables["App::Plain"].attribute_columns["name"].as_str(),
        "n"
    );
    assert_eq!(
        c.tables["App::Qualified"].attribute_columns["name"].as_str(),
        "n"
    );
    assert_eq!(
        c.tables["App::Cross"].attribute_columns["name"].as_str(),
        "lib_name"
    );
}

#[test]
fn long_default_names() {
    let long = "A".repeat(70);
    let src = format!(
        "entity {long} = {{ {long}: String }} tags String; action a appliesTo {{ principal: [{long}], resource: [{long}] }};"
    );
    let e = err(&src);
    assert!(e.contains("use @sql_table"), "{e}");
    let (_, schema) = DatabaseConfiguration::from_cedarschema_str(
        "entity User; action a appliesTo { principal: [User], resource: [User] };",
    )
    .unwrap();
    let _ = schema;
    let schema = cedar_policy::Schema::from_cedarschema_str(&src).unwrap().0;
    let c = DatabaseConfiguration::from_schema_shortening_names(&schema).unwrap();
    let (name, table) = c.table_for(&long.parse().unwrap()).unwrap();
    assert!(name.as_str().len() <= 63 && name.as_str().starts_with("AAAA"));
    assert!(table.tags.as_ref().unwrap().table.as_str().len() <= 63);
    assert!(table.attribute_columns[long.as_str()].as_str().len() <= 63);
}

#[test]
fn json_schema_annotations() {
    let (c, _) = DatabaseConfiguration::from_json_str(
        r#"{"": {"entityTypes": {"User": {"annotations": {"sql_table": "users"}, "shape": {"type": "Record", "attributes": {"n": {"type": "String", "annotations": {"sql_column": "name"}}}}}}, "actions": {"a": {"appliesTo": {"principalTypes": ["User"], "resourceTypes": ["User"]}}}}}"#,
    )
    .unwrap();
    assert!(c.tables.contains_key("users"));
    assert_eq!(c.tables["users"].attribute_columns["n"].as_str(), "name");
}
