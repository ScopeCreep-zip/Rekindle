use super::*;
use crate::repo::fixture::{self, OWNER};

#[test]
fn a_peer_record_is_kept_for_an_accepted_friend_only() {
    let conn = fixture::conn();
    conn.execute(
        "INSERT INTO friends (owner_key, public_key, added_at, friendship_state) \
             VALUES (?1, 'acc', 0, 'accepted'), (?1, 'pend', 0, 'pending_out')",
        params![OWNER],
    )
    .unwrap();
    assert!(set_peer_participant_record_key(&conn, OWNER, "acc", "r1").unwrap());
    assert!(!set_peer_participant_record_key(&conn, OWNER, "acc", "r1").unwrap());
    assert!(!set_peer_participant_record_key(&conn, OWNER, "pend", "r2").unwrap());
    assert_eq!(
        peer_participant_record_key(&conn, OWNER, None).unwrap(),
        [("acc".to_string(), "r1".to_string())]
    );
    assert!(set_participant_record_key(&conn, OWNER, "pend", "ours").unwrap());
    assert_eq!(
        participant_record_key(&conn, OWNER, "pend")
            .unwrap()
            .as_deref(),
        Some("ours")
    );
    assert_eq!(participant_record_key(&conn, OWNER, "none").unwrap(), None);
}

#[test]
fn a_new_record_starts_a_new_chain() {
    let conn = fixture::conn();
    conn.execute(
        "INSERT INTO friends (owner_key, public_key, added_at, friendship_state) \
             VALUES (?1, 'bob', 0, 'accepted')",
        params![OWNER],
    )
    .unwrap();
    assert!(set_participant_record_key(&conn, OWNER, "bob", "old").unwrap());
    crate::repo::chain_entries::record(&conn, OWNER, "bob", 1, 1, 0).unwrap();
    // The same record again keeps its chain.
    assert!(set_participant_record_key(&conn, OWNER, "bob", "old").unwrap());
    let count = |c: &Connection| -> i64 {
        c.query_row("SELECT COUNT(*) FROM chain_entries", [], |r| r.get(0))
            .unwrap()
    };
    assert_eq!(count(&conn), 1);
    // A new one forgets the old chain's entries, so its seq 1 is noted.
    assert!(set_participant_record_key(&conn, OWNER, "bob", "new").unwrap());
    assert_eq!(count(&conn), 0);
    crate::repo::chain_entries::record(&conn, OWNER, "bob", 1, 9, 0).unwrap();
    let seq: i64 = conn
        .query_row(
            "SELECT stream_seq FROM chain_entries WHERE chain_seq = 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(seq, 9);
}
