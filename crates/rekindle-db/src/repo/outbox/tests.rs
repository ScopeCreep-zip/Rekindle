use rekindle_types::message::SendState;
use rusqlite::Connection;

use super::*;
use crate::repo::fixture::{self, OWNER};
use crate::repo::messages::{own_dm, NewDm};

fn row<'a>(stream: &'a str, client_id: &'a str, next_at: i64) -> NewRow<'a> {
    NewRow {
        owner_key: OWNER,
        stream,
        client_id,
        kind: "dm",
        payload: b"frame",
        next_at,
        deadline: 1_000,
    }
}

fn echo() -> NewDm<'static> {
    NewDm {
        owner_key: OWNER,
        peer_key: "bob",
        conversation_type: rekindle_types::message::ConversationType::Dm,
        sender_key: OWNER,
        body: "hi",
        timestamp: 1,
        stream_seq: None,
        is_read: true,
        message_id: "ignored",
        reply_to: None,
        send_state: SendState::Delivered,
    }
}

fn sent(attempts: u32, next_at: i64) -> Update<'static> {
    Update {
        state: SendState::Sent,
        attempts,
        next_at,
        deadline: 1_000,
        wedge_reason: None,
    }
}

#[test]
fn the_row_and_its_echo_are_one_write_and_seqs_never_repeat() {
    let conn = fixture::conn();
    assert_eq!(
        enqueue_with_echo(&conn, &row("dm:bob", "a", 0), Some(&echo())).unwrap(),
        1
    );
    let echoed = own_dm(&conn, OWNER, "a").unwrap().unwrap();
    assert_eq!(echoed.row.send_state, SendState::Queued);
    assert!(settle(&conn, OWNER, "a", SendState::Delivered).unwrap());
    assert_eq!(
        own_dm(&conn, OWNER, "a").unwrap().unwrap().row.send_state,
        SendState::Delivered
    );
    // The settled row is gone; the next one still gets a fresh seq.
    assert_eq!(
        enqueue_with_echo(&conn, &row("dm:bob", "b", 0), None).unwrap(),
        2
    );
    assert_eq!(
        enqueue_with_echo(&conn, &row("dm:carol", "c", 0), None).unwrap(),
        1
    );
    // A failed echo insert (duplicate client id) stores no row.
    assert!(enqueue_with_echo(&conn, &row("dm:bob", "b", 0), None).is_err());
    assert_eq!(unsettled_count(&conn, OWNER).unwrap(), 2);
}

#[test]
fn the_stream_head_blocks_the_rows_behind_it() {
    let conn = fixture::conn();
    enqueue_with_echo(&conn, &row("dm:bob", "a", 50), None).unwrap();
    enqueue_with_echo(&conn, &row("dm:bob", "b", 0), None).unwrap();
    enqueue_with_echo(&conn, &row("dm:carol", "c", 0), None).unwrap();
    let ids = |now| -> Vec<String> {
        due(&conn, OWNER, now)
            .unwrap()
            .into_iter()
            .map(|r| r.client_id)
            .collect()
    };
    assert_eq!(ids(10), ["c"]);
    assert_eq!(next_wake(&conn, OWNER).unwrap(), Some(0));
    update(&conn, OWNER, "a", &sent(1, 500)).unwrap();
    assert_eq!(ids(10), ["b", "c"]);
    // A wedged head blocks too, until it is discarded.
    update(
        &conn,
        OWNER,
        "a",
        &Update {
            state: SendState::Failed,
            wedge_reason: Some("deadline"),
            ..sent(1, 500)
        },
    )
    .unwrap();
    assert_eq!(ids(10), ["c"]);
    settle(&conn, OWNER, "a", SendState::Failed).unwrap();
    assert_eq!(ids(10), ["b", "c"]);
}

#[test]
fn expired_rows_and_lookups_by_seq() {
    let conn = fixture::conn();
    enqueue_with_echo(&conn, &row("dm:bob", "a", 0), None).unwrap();
    assert!(expired(&conn, OWNER, 999).unwrap().is_empty());
    assert_eq!(expired(&conn, OWNER, 1_000).unwrap()[0].client_id, "a");
    assert_eq!(
        get_by_seq(&conn, OWNER, "dm:bob", 1)
            .unwrap()
            .unwrap()
            .client_id,
        "a"
    );
    assert!(get(&conn, OWNER, "zz").unwrap().is_none());
    assert!(update(&conn, OWNER, "zz", &sent(1, 0)).unwrap().is_none());
}

#[test]
fn a_resend_is_received_once() {
    let conn = fixture::conn();
    assert!(mark_received(&conn, OWNER, "dm:bob", "a").unwrap());
    assert!(!mark_received(&conn, OWNER, "dm:bob", "a").unwrap());
    assert!(mark_received(&conn, OWNER, "dm:carol", "a").unwrap());
}

#[test]
fn rows_survive_a_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("outbox.db");
    {
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
        conn.execute_batch(crate::SCHEMA).unwrap();
        crate::repo::identity::insert(&conn, OWNER, "me", 0, "aaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap();
        enqueue_with_echo(&conn, &row("dm:bob", "a", 0), Some(&echo())).unwrap();
    }
    let conn = Connection::open(&path).unwrap();
    let rows = due(&conn, OWNER, 0).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(
        (rows[0].client_id.as_str(), rows[0].state),
        ("a", SendState::Queued)
    );
    assert_eq!(rows[0].payload, b"frame");
}

#[test]
fn read_through_raises_settled_and_waiting_echoes_at_or_below_the_marker() {
    let conn = fixture::conn();
    for id in ["a", "b", "c", "d"] {
        enqueue_with_echo(&conn, &row("dm:bob", id, 0), Some(&echo())).unwrap();
    }
    // `a` was delivered and settled; `b` landed and waits; `c` has not
    // landed (the peer read its hint first); `d` is past the marker.
    assert!(settle(&conn, OWNER, "a", SendState::Delivered).unwrap());
    update(&conn, OWNER, "b", &sent(1, 0)).unwrap().unwrap();
    let raised = read_through(&conn, OWNER, "dm:bob", "bob", 3).unwrap();
    assert_eq!(raised, [(1, "a".to_owned()), (2, "b".to_owned())]);
    let state = |id| own_dm(&conn, OWNER, id).unwrap().unwrap().row.send_state;
    assert_eq!((state("a"), state("b")), (SendState::Read, SendState::Read));
    assert_eq!(state("c"), SendState::Queued, "not landed: marked, kept");
    assert_eq!(state("d"), SendState::Queued, "past the marker");
    assert_eq!(unsettled_count(&conn, OWNER).unwrap(), 2);
    assert!(read_through(&conn, OWNER, "dm:bob", "bob", 2)
        .unwrap()
        .is_empty());
    // `c`'s landing reads the peer's read back (live A/B finding 42
    // follow-up); `d`'s finds none.
    let landed = update(&conn, OWNER, "c", &sent(1, 0)).unwrap().unwrap();
    assert_eq!(landed.peer_ack, Some(SendState::Read));
    let landed = update(&conn, OWNER, "d", &sent(1, 0)).unwrap().unwrap();
    assert_eq!(landed.peer_ack, None);
}

#[test]
fn a_received_message_keeps_the_senders_stream_seq() {
    let conn = fixture::conn();
    let theirs = NewDm {
        sender_key: "bob",
        is_read: false,
        message_id: "m1",
        stream_seq: Some(7),
        ..echo()
    };
    crate::repo::messages::insert_dm(&conn, &theirs).unwrap();
    let seq = |id| crate::repo::messages::received_stream_seq(&conn, OWNER, "bob", id).unwrap();
    assert_eq!(seq("m1"), Some(7));
    assert_eq!(seq("unknown"), None);
}

#[test]
fn prefix_rows_are_one_sessions_streams_only() {
    let conn = fixture::conn();
    for (stream, id) in [
        ("media_key:c/ch/bob", "a"),
        ("media_key:c/ch/carol", "b"),
        ("media_key:c/ch2/bob", "c"),
        ("dm:bob", "d"),
    ] {
        enqueue_with_echo(&conn, &row(stream, id, 0), None).unwrap();
    }
    let ids: Vec<String> = (prefix_rows(&conn, OWNER, "media_key:c/ch/").unwrap())
        .into_iter()
        .map(|r| r.client_id)
        .collect();
    assert_eq!(ids, ["a", "b"]);
}

/// E2.B F3: a row carries its echo's time, which each attempt's frame
/// sends, and an echo is never written before our earlier messages in its
/// conversation (a clock that went back), so our history keeps stream order.
#[test]
fn a_row_carries_when_its_echo_was_written_and_times_follow_the_stream() {
    let conn = fixture::conn();
    let at = |timestamp| NewDm {
        timestamp,
        ..echo()
    };
    enqueue_with_echo(&conn, &row("dm:bob", "a", 0), Some(&at(500))).unwrap();
    enqueue_with_echo(&conn, &row("dm:bob", "b", 0), Some(&at(300))).unwrap();
    enqueue_with_echo(&conn, &row("dm:bob", "c", 0), None).unwrap();
    let sent_at = |id| get(&conn, OWNER, id).unwrap().unwrap().sent_at;
    assert_eq!(sent_at("a"), Some(500));
    assert_eq!(sent_at("b"), Some(500), "never behind the earlier echo");
    assert_eq!(sent_at("c"), None);
    assert_eq!(
        own_dm(&conn, OWNER, "b").unwrap().unwrap().row.timestamp,
        500
    );
}

/// Live A/B finding 42: an ack before the landing is kept on the queued
/// row (the higher one stays) and read back by the landing's update; a
/// row past `queued` records none.
#[test]
fn an_ack_before_the_landing_is_kept_until_the_update_reads_it() {
    let conn = fixture::conn();
    enqueue_with_echo(&conn, &row("dm:bob", "a", 0), None).unwrap();
    assert!(note_ack(&conn, OWNER, "a", SendState::Read).unwrap());
    assert!(note_ack(&conn, OWNER, "a", SendState::Delivered).unwrap());
    assert_eq!(
        update(&conn, OWNER, "a", &sent(1, 0)).unwrap(),
        Some(Updated {
            peer_ack: Some(SendState::Read)
        })
    );
    assert!(!note_ack(&conn, OWNER, "a", SendState::Delivered).unwrap());
    enqueue_with_echo(&conn, &row("dm:bob", "b", 0), None).unwrap();
    assert_eq!(
        update(&conn, OWNER, "b", &sent(1, 0)).unwrap(),
        Some(Updated { peer_ack: None })
    );
}
