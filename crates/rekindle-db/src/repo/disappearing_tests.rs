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

fn change<'a>(
    conversation: &'a str,
    seconds: u32,
    set_by: &'a str,
    now: i64,
    id: &'a str,
) -> TimerChange<'a> {
    TimerChange {
        owner_key: OWNER,
        conversation,
        kind: ConversationType::Dm,
        seconds,
        set_by,
        now,
        notice_id: id,
    }
}

#[test]
fn the_timer_is_per_conversation() {
    let conn = seeded();
    assert_eq!(timer(&conn, OWNER, "bob", ConversationType::Dm).unwrap(), 0);
    assert_eq!(
        change_timer(&conn, &change("bob", 30, OWNER, 0, "n1")).unwrap(),
        TimerChanged::Changed
    );
    assert_eq!(
        timer(&conn, OWNER, "bob", ConversationType::Dm).unwrap(),
        30
    );
    assert_eq!(
        change_timer(&conn, &change("carol", 30, OWNER, 0, "n2")).unwrap(),
        TimerChanged::NotHeld
    );
}

/// Plan E7.1 (gap from the UI slice): a timer change is a stored timeline
/// row, so a reopened window shows past changes. An unchanged value
/// stores none; the notice disappears on the timer it set (ours from now,
/// a received one from display), and one turning the timer off stays.
#[test]
fn a_timer_change_is_a_stored_notice_that_disappears_on_its_own_timer() {
    use crate::repo::messages::{dm_history, dm_inbox};
    let conn = seeded();
    insert_dm(&conn, &dm("bob", "hello", 0, None)).unwrap();
    assert_eq!(
        change_timer(&conn, &change("bob", 30, OWNER, 2_000, "ours")).unwrap(),
        TimerChanged::Changed
    );
    assert_eq!(
        change_timer(&conn, &change("bob", 30, "bob", 3_000, "dup")).unwrap(),
        TimerChanged::Unchanged
    );
    change_timer(&conn, &change("bob", 60, "bob", 4_000, "theirs")).unwrap();
    change_timer(&conn, &change("bob", 0, "bob", 5_000, "off")).unwrap();

    let history = dm_history(&conn, OWNER, "bob", 10, None).unwrap();
    let notices: Vec<_> = history
        .iter()
        .filter_map(|r| Some((r.message_id.clone()?, r.timer_notice_seconds?, r.expire_at)))
        .collect();
    assert_eq!(
        notices,
        [
            ("ours".to_owned(), 30, Some(32_000)),
            ("theirs".to_owned(), 60, None),
            ("off".to_owned(), 0, None),
        ]
    );
    // The inbox's latest message is never a notice.
    let inbox = dm_inbox(&conn, OWNER, 10).unwrap();
    assert_eq!(inbox[0].message_id.as_deref(), Some("hello"));

    assert_eq!(start_on_display(&conn, OWNER, "bob", 10_000).unwrap(), 1);
    assert_eq!(
        delete_expired(&conn, OWNER, 32_000).unwrap()[0].client_ids,
        ["ours"]
    );
    assert_eq!(
        delete_expired(&conn, OWNER, 70_000).unwrap()[0].client_ids,
        ["theirs"]
    );
    let left: Vec<String> = conn
        .prepare("SELECT message_id FROM messages ORDER BY id")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(left, ["hello", "off"]);
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
