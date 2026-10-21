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
        sender_key: OWNER,
        body: "hi",
        timestamp: 1,
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
    assert!(!update(&conn, OWNER, "zz", &sent(1, 0)).unwrap());
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
