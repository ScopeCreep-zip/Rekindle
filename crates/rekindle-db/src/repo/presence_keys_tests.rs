use super::{current, replace};

#[test]
fn the_newest_epoch_is_the_only_one_kept() {
    let conn = crate::test_support::schema_conn("aa");
    assert_eq!(current(&conn, "aa").unwrap(), None);
    replace(&conn, "aa", 1, &[1; 32], 10).unwrap();
    replace(&conn, "aa", 2, &[2; 32], 20).unwrap();
    assert_eq!(current(&conn, "aa").unwrap(), Some((2, vec![2; 32])));
    let rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM presence_keys", [], |r| r.get(0))
        .unwrap();
    assert_eq!(rows, 1);
    assert!(
        replace(&conn, "aa", 3, &[3; 16], 30).is_err(),
        "a key is 32 bytes"
    );
}
