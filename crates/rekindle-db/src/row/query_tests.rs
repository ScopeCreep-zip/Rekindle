use super::*;

#[derive(Debug, PartialEq)]
struct Pair {
    name: String,
    at: i64,
}

impl FromRow for Pair {
    fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            name: row.get(0)?,
            at: row.get(1)?,
        })
    }
}

fn table() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE t (name TEXT, at INTEGER);
         INSERT INTO t VALUES ('a', 1), ('b', 2), (NULL, 3);",
    )
    .unwrap();
    conn
}

#[test]
fn a_list_binds_its_parameters_and_keeps_the_query_order() {
    let conn = table();
    let rows: Vec<Pair> = query_list(
        &conn,
        "SELECT name, at FROM t WHERE at <= ?1 ORDER BY at DESC",
        [2],
    )
    .unwrap();
    let want = [("b", 2), ("a", 1)].map(|(name, at)| Pair {
        name: name.into(),
        at,
    });
    assert_eq!(rows, want);
    let tuples: Vec<(String, i64)> =
        query_list(&conn, "SELECT name, at FROM t WHERE at = 1", []).unwrap();
    assert_eq!(tuples, [("a".to_owned(), 1)]);
}

#[test]
fn a_malformed_row_is_an_error_never_a_default() {
    let conn = table();
    assert!(query_list::<Pair>(&conn, "SELECT name, at FROM t", []).is_err());
    assert!(query_one::<Pair>(&conn, "SELECT name, at FROM t WHERE at = 3", []).is_err());
    assert!(query_opt::<Pair>(&conn, "SELECT name, at FROM t WHERE at = 3", []).is_err());
}

#[test]
fn one_requires_a_row_and_opt_reads_none() {
    let conn = table();
    let missing = "SELECT name, at FROM t WHERE at = 9";
    assert!(matches!(
        query_one::<Pair>(&conn, missing, []),
        Err(rusqlite::Error::QueryReturnedNoRows)
    ));
    assert_eq!(query_opt::<Pair>(&conn, missing, []).unwrap(), None);
    assert_eq!(
        query_one::<String>(&conn, "SELECT name FROM t WHERE at = 2", []).unwrap(),
        "b"
    );
}
