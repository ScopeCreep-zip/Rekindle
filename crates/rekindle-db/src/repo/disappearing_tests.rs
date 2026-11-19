use rusqlite::params;

use super::*;
use crate::repo::fixture::{self, OWNER};
use crate::repo::messages::{insert_dm, NewDm};
use rekindle_types::message::SendState;

fn seeded() -> Connection {
    let conn = fixture::conn();
    conn.execute(
        "INSERT INTO friends (owner_key, public_key, added_at, friendship_state) \
         VALUES (?1, 'bob', 0, 'accepted')",
        params![OWNER],
    )
    .unwrap();
    conn
}

fn dm<'a>(sender: &'a str, id: &'a str, secs: u32, expire_at: Option<i64>) -> NewDm<'a> {
    NewDm {
        owner_key: OWNER,
        peer_key: "bob",
        conversation_type: ConversationType::Dm,
        expire_seconds: secs,
        expire_at,
        sender_key: sender,
        body: "hi",
        timestamp: 1_000,
        is_read: sender == OWNER,
        message_id: id,
        reply_to: None,
        send_state: SendState::Delivered,
        stream_seq: None,
    }
}

#[test]
fn the_timer_is_per_conversation() {
    let conn = seeded();
    assert_eq!(timer(&conn, OWNER, "bob", ConversationType::Dm).unwrap(), 0);
    assert!(set_timer(&conn, OWNER, "bob", ConversationType::Dm, 30).unwrap());
    assert_eq!(
        timer(&conn, OWNER, "bob", ConversationType::Dm).unwrap(),
        30
    );
    assert!(!set_timer(&conn, OWNER, "carol", ConversationType::Dm, 30).unwrap());
}

#[test]
fn a_received_timer_starts_at_display_and_expiry_deletes_the_rows_and_their_sends() {
    let conn = seeded();
    insert_dm(&conn, &dm(OWNER, "mine", 30, Some(31_000))).unwrap();
    insert_dm(&conn, &dm("bob", "theirs", 60, None)).unwrap();
    insert_dm(&conn, &dm("bob", "kept", 0, None)).unwrap();
    conn.execute(
        "INSERT INTO outbox (owner_key, stream, stream_seq, client_id, kind, payload, state, \
         next_at, deadline) VALUES (?1, 'dm:bob', 1, 'mine', 'dm', x'00', 'sent', 0, 0)",
        params![OWNER],
    )
    .unwrap();
    // Unread, the received one's timer has not started.
    assert_eq!(next_expiry(&conn, OWNER).unwrap(), Some(31_000));
    assert_eq!(start_on_display(&conn, OWNER, "bob", 5_000).unwrap(), 1);
    assert_eq!(start_on_display(&conn, OWNER, "bob", 9_000).unwrap(), 0);

    let gone = delete_expired(&conn, OWNER, 31_000).unwrap();
    assert_eq!(
        gone,
        vec![Expired {
            conversation_id: "bob".into(),
            conversation_type: ConversationType::Dm,
            client_ids: vec!["mine".into()],
        }]
    );
    let outbox: i64 = conn
        .query_row("SELECT count(*) FROM outbox", [], |r| r.get(0))
        .unwrap();
    assert_eq!(outbox, 0);
    assert_eq!(next_expiry(&conn, OWNER).unwrap(), Some(65_000));
    assert_eq!(
        delete_expired(&conn, OWNER, 65_000).unwrap()[0].client_ids,
        ["theirs"]
    );
    let left: Vec<String> = conn
        .prepare("SELECT message_id FROM messages")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(left, ["kept"]);
    assert_eq!(next_expiry(&conn, OWNER).unwrap(), None);
}

/// Plan E7.1: an expired message's logged plaintext and a leftover
/// `unprocessed` row of it go in the same transaction; others stay.
#[test]
fn expiry_deletes_the_send_log_and_unprocessed_rows_of_its_messages() {
    let conn = seeded();
    insert_dm(&conn, &dm(OWNER, "mine", 30, Some(31_000))).unwrap();
    insert_dm(&conn, &dm("bob", "theirs", 60, Some(31_000))).unwrap();
    for (n, id) in [(1u8, "mine"), (2, "other")] {
        conn.execute(
            "INSERT INTO message_send_log (owner_key, peer, msg_id, type_id, ratchet_key, \
             plaintext_sealed, sent_at, client_id) \
             VALUES (?1, 'bob', ?2, 256, zeroblob(32), x'00', 0, ?3)",
            params![OWNER, [n; 32].as_slice(), id],
        )
        .unwrap();
    }
    for (n, id) in [(1, "theirs"), (2, "other")] {
        conn.execute(
            "INSERT INTO unprocessed (owner_key, peer_identity, header_dh, header_n, type_id, \
             plaintext, received_at, client_id) VALUES (?1, 'bob', zeroblob(32), ?2, 256, \
             x'00', 0, ?3)",
            params![OWNER, n, id],
        )
        .unwrap();
    }
    delete_expired(&conn, OWNER, 31_000).unwrap();
    for table in ["message_send_log", "unprocessed"] {
        let left: Vec<String> = conn
            .prepare(&format!("SELECT client_id FROM {table}"))
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(left, ["other"], "{table}");
    }
}
