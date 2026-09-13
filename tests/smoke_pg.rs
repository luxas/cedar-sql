//! The provisioning smoke test: a query round trip through the shared Postgres.

use cedar_sql::backend::{Backend, SqlValue};
use cedar_sql::testing::SharedPostgres;

#[test]
fn select_one() {
    let pg = SharedPostgres::get().expect("a Postgres is available");
    let mut db = pg.connect().expect("connects");
    db.begin().unwrap();
    db.execute_batch("CREATE TABLE t (id TEXT PRIMARY KEY, n BIGINT, b BOOLEAN, j JSONB)")
        .unwrap();
    db.execute_batch("INSERT INTO t VALUES ('a', 1, true, '[1, {\"r\": {}}]')")
        .unwrap();
    let rows = db.query("SELECT id, n, b, j, NULL::bigint FROM t").unwrap();
    assert_eq!(
        rows,
        vec![vec![
            SqlValue::Text("a".into()),
            SqlValue::Long(1),
            SqlValue::Bool(true),
            SqlValue::Json(serde_json::json!([1, {"r": {}}])),
            SqlValue::Null,
        ]]
    );
    db.rollback().unwrap();
    // The table did not survive the rollback.
    assert!(db.query("SELECT 1 FROM t").is_err());
}
