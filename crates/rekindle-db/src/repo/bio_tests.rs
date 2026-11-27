use super::*;
use crate::repo::fixture::{conn, OWNER};
use crate::repo::{friends, outbox};

#[test]
fn our_bio_is_kept_and_refused_past_190_characters() {
    let conn = conn();
    assert_eq!(own(&conn, OWNER).unwrap(), None);
    set_own(&conn, OWNER, Some("hi")).unwrap();
    assert_eq!(own(&conn, OWNER).unwrap().as_deref(), Some("hi"));
    assert!(set_own(&conn, OWNER, Some(&"é".repeat(190))).is_ok());
    assert!(set_own(&conn, OWNER, Some(&"x".repeat(191))).is_err());
    assert!(set_own(&conn, OWNER, Some("")).is_err());
    set_own(&conn, OWNER, None).unwrap();
    assert_eq!(own(&conn, OWNER).unwrap(), None);
}

#[test]
fn a_friends_bio_is_kept_and_removing_them_clears_it() {
    let conn = conn();
    conn.execute(
        "INSERT INTO friends (owner_key, public_key, added_at, friendship_state) \
             VALUES (?1, 'bob', 0, 'accepted')",
        params![OWNER],
    )
    .unwrap();
    assert!(set_friend(&conn, OWNER, "bob", Some("gg")).unwrap());
    assert!(!set_friend(&conn, OWNER, "carol", Some("gg")).unwrap());
    assert_eq!(friend(&conn, OWNER, "bob").unwrap().as_deref(), Some("gg"));
    let row = outbox::NewRow {
        owner_key: OWNER,
        stream: "friend_handshake:bob",
        client_id: "unfriended:bob",
        kind: "friend_handshake",
        payload: b"{}",
        next_at: 2,
        deadline: 9,
        depends_on: None,
    };
    friends::mark_removing(&conn, OWNER, "bob", &row).unwrap();
    assert_eq!(friend(&conn, OWNER, "bob").unwrap(), None);
}
