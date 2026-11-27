use super::*;
use crate::repo::fixture::{self, OWNER};

const G: &str = "0123456789abcdef0123456789abcdef";

fn row(at: i64) -> InviteRow {
    InviteRow {
        group_id: G.to_string(),
        from_key: "a".repeat(64),
        invite: vec![1, 2, 3],
        received_at: at,
    }
}

#[test]
fn an_invite_is_held_replaced_and_taken_once() {
    let conn = fixture::conn();
    put(&conn, OWNER, &row(5)).unwrap();
    put(&conn, OWNER, &row(6)).unwrap();
    assert_eq!(take(&conn, OWNER, G, 7).unwrap(), Some(row(6)));
    assert_eq!(take(&conn, OWNER, G, 7).unwrap(), None, "taken once");
    let mut bad = row(5);
    bad.from_key = "short".into();
    assert!(put(&conn, OWNER, &bad).is_err());
}

#[test]
fn an_invite_past_the_horizon_is_gone() {
    let conn = fixture::conn();
    let late = max_age_ms() + 10;
    put(&conn, OWNER, &row(5)).unwrap();
    assert_eq!(take(&conn, OWNER, G, late).unwrap(), None);
    put(&conn, OWNER, &row(5)).unwrap();
    assert_eq!(prune(&conn, OWNER, late).unwrap(), 1);
    put(&conn, OWNER, &row(5)).unwrap();
    assert_eq!(prune(&conn, OWNER, 6).unwrap(), 0);
}
