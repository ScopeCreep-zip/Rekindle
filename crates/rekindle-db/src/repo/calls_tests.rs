use super::{list_missed, record_missed};

#[test]
fn a_missed_call_is_recorded_once_and_listed_newest_first() {
    let conn = crate::repo::fixture::conn();
    let owner = crate::repo::fixture::OWNER;
    let peer = "b".repeat(64);
    record_missed(&conn, owner, "c1", &peer, 0, 10).unwrap();
    record_missed(&conn, owner, "c1", &peer, 0, 10).unwrap();
    record_missed(&conn, owner, "c2", &peer, 1, 20).unwrap();
    let rows = list_missed(&conn, owner).unwrap();
    let ids: Vec<&str> = rows.iter().map(|r| r.call_id.as_str()).collect();
    assert_eq!(ids, ["c2", "c1"]);
    assert_eq!(rows[0].kind, 1);
    assert!(list_missed(&conn, &"c".repeat(64)).unwrap().is_empty());
}
