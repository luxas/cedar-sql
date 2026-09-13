//! The DDL of the README's schemas, against golden files (regenerate with
//! `UPDATE_GOLDEN=1`), and executed on Postgres.

use cedar_sql::backend::Backend;
use cedar_sql::config::DatabaseConfiguration;
use cedar_sql::ddl::{create_tables, drop_tables};
use cedar_sql::dialect::Postgres;
use cedar_sql::testing::SharedPostgres;

fn check(name: &str) -> Vec<String> {
    let src = std::fs::read_to_string(format!("tests/schemas/{name}.cedarschema")).unwrap();
    let (config, _schema) = DatabaseConfiguration::from_cedarschema_str(&src).unwrap();
    let statements = create_tables(&config, &Postgres).unwrap();
    let rendered = statements
        .iter()
        .map(|s| format!("{s};\n"))
        .collect::<String>();
    let golden = format!("tests/golden/{name}.sql");
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::write(&golden, &rendered).unwrap();
    }
    let expected = std::fs::read_to_string(&golden)
        .unwrap_or_else(|e| panic!("{golden}: {e}; run with UPDATE_GOLDEN=1"));
    assert_eq!(
        rendered, expected,
        "{golden} differs; run with UPDATE_GOLDEN=1 to update"
    );

    // The DDL runs, and so does its drop.
    let mut db = SharedPostgres::get().unwrap().connect().unwrap();
    db.begin().unwrap();
    for statement in &statements {
        db.execute_batch(statement)
            .unwrap_or_else(|e| panic!("{statement}\n{e}"));
    }
    for statement in drop_tables(&config) {
        db.execute_batch(&statement).unwrap();
    }
    db.rollback().unwrap();
    statements
}

#[test]
fn readme_simple() {
    check("readme_simple");
}

#[test]
fn readme_annotated() {
    check("readme_annotated");
}

#[test]
fn kitchen_sink() {
    check("kitchen_sink");
}

#[test]
fn pk_and_refs() {
    let statements = check("pk_and_refs");
    // The shortened constraint names are within the limit and distinct.
    let names: Vec<&str> = statements
        .iter()
        .filter_map(|s| s.split("ADD CONSTRAINT \"").nth(1))
        .map(|s| s.split('"').next().unwrap())
        .collect();
    assert_eq!(names.len(), 4);
    assert!(names.iter().all(|n| n.len() <= 63), "{names:?}");
    assert_eq!(
        names
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        4
    );
}
