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
    assert_eq!(
        raised,
        [
            (1, "a".to_owned()),
            (2, "b".to_owned()),
            (3, "c".to_owned())
        ]
    );
    let state = |id| own_dm(&conn, OWNER, id).unwrap().unwrap().row.send_state;
    assert_eq!((state("a"), state("b")), (SendState::Read, SendState::Read));
    // D22: `c` shows read at once; not landed, its row is kept.
    assert_eq!(state("c"), SendState::Read, "not landed: shown read, kept");
    assert!(
        read_through(&conn, OWNER, "dm:bob", "bob", 3)
            .unwrap()
            .is_empty(),
        "raised once"
    );
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

fn at(state: SendState) -> Update<'static> {
    Update {
        state,
        attempts: 1,
        next_at: 0,
        deadline: 1_000,
        wedge_reason: None,
    }
}

/// LV17 follow-up (i): the echo's state is monotone, merged with max (D22).
/// A re-handed-off in-flight row is `queued` again, but its delivered echo
/// stays delivered; a `failed` echo is queued again by a retry.
#[test]
fn the_echo_state_never_goes_down() {
    let conn = fixture::conn();
    let state = |id: &str| own_dm(&conn, OWNER, id).unwrap().unwrap().row.send_state;
    enqueue_with_echo(&conn, &row("dm:bob", "a", 0), Some(&echo())).unwrap();
    assert!(note_ack(&conn, OWNER, "a", SendState::Delivered).unwrap());
    assert_eq!(state("a"), SendState::Delivered);
    for lower in [SendState::Queued, SendState::Sent, SendState::Failed] {
        update(&conn, OWNER, "a", &at(lower)).unwrap();
        assert_eq!(state("a"), SendState::Delivered, "{lower:?} lowered it");
    }
    update(&conn, OWNER, "a", &at(SendState::Read)).unwrap();
    assert_eq!(state("a"), SendState::Read);
    assert!(settle(&conn, OWNER, "a", SendState::Delivered).unwrap());
    assert_eq!(state("a"), SendState::Read);

    let mut b = echo();
    b.message_id = "b";
    enqueue_with_echo(&conn, &row("dm:bob", "b", 0), Some(&b)).unwrap();
    update(&conn, OWNER, "b", &at(SendState::Sent)).unwrap();
    update(&conn, OWNER, "b", &at(SendState::Queued)).unwrap();
    assert_eq!(state("b"), SendState::Sent, "a re-handoff keeps sent");
    update(&conn, OWNER, "b", &at(SendState::Failed)).unwrap();
    assert_eq!(state("b"), SendState::Failed);
    update(&conn, OWNER, "b", &at(SendState::Queued)).unwrap();
    assert_eq!(
        state("b"),
        SendState::Queued,
        "a retry queues a failed echo"
    );
}

/// Review RV3 (d3bcd308): a cancelled request or a session's end discards
/// every row still on the stream, a `sent` one and a wedged `failed` one
/// awaiting Retry as well as a `queued` one.
#[test]
fn prefix_rows_are_every_row_still_on_the_streams() {
    let conn = fixture::conn();
    for id in ["a", "b", "c"] {
        enqueue_with_echo(&conn, &row("friend_handshake:bob", id, 0), None).unwrap();
    }
    update(&conn, OWNER, "b", &sent(1, 500)).unwrap();
    let wedged = Update {
        state: SendState::Failed,
        wedge_reason: Some("refused"),
        ..sent(1, 500)
    };
    update(&conn, OWNER, "c", &wedged).unwrap();
    let ids: Vec<String> = (prefix_rows(&conn, OWNER, "friend_handshake:bob").unwrap())
        .into_iter()
        .map(|r| r.client_id)
        .collect();
    assert_eq!(ids, ["a", "b", "c"]);
}

/// Review RV3 (151be550): hearing from the peer again brings each landed
/// row's re-ask forward to `at`, never later and never a queued row's.
#[test]
fn reschedule_sent_brings_only_landed_rows_forward() {
    let conn = fixture::conn();
    for id in ["late", "soon", "queued"] {
        enqueue_with_echo(&conn, &row("dm:bob", id, 900), None).unwrap();
    }
    update(&conn, OWNER, "late", &sent(1, 500)).unwrap();
    update(&conn, OWNER, "soon", &sent(1, 50)).unwrap();
    enqueue_with_echo(&conn, &row("dm:carol", "other", 0), None).unwrap();
    update(&conn, OWNER, "other", &sent(1, 500)).unwrap();
    assert_eq!(reschedule_sent(&conn, OWNER, "dm:bob", 100).unwrap(), 1);
    let next_at = |id| get(&conn, OWNER, id).unwrap().unwrap().next_at;
    assert_eq!(
        [
            next_at("late"),
            next_at("soon"),
            next_at("queued"),
            next_at("other")
        ],
        [100, 50, 900, 500]
    );
}

fn chain(conn: &Connection, chain_seq: i64, stream_seq: i64) {
    conn.execute(
        "INSERT INTO chain_entries (owner_key, peer, chain_seq, stream_seq) VALUES (?1, 'bob', ?2, ?3)",
        rusqlite::params![OWNER, chain_seq, stream_seq],
    )
    .unwrap();
}

/// Review RV3 (f61572a0): `through` is a chain index, mapped to stream
/// seqs over `chain_entries`: a landed row it covers settles, a queued one
/// keeps its row with its echo raised, and a row past it or not on the
/// chain (a StreamRecord) is untouched.
#[test]
fn deliver_chain_through_maps_chain_seqs_to_stream_rows() {
    let conn = fixture::conn();
    for id in ["record", "c1", "c2", "c3"] {
        enqueue_with_echo(&conn, &row("dm:bob", id, 0), Some(&echo())).unwrap();
    }
    // Stream seqs 1..4; the chain starts at the second.
    for (chain_seq, stream_seq) in [(1, 2), (2, 3), (3, 4)] {
        chain(&conn, chain_seq, stream_seq);
    }
    for id in ["record", "c1", "c3"] {
        update(&conn, OWNER, id, &sent(1, 500)).unwrap();
    }
    let raised = deliver_chain_through(&conn, OWNER, "dm:bob", "bob", 2).unwrap();
    let mut raised: Vec<i64> = raised.into_iter().map(|(seq, _)| seq).collect();
    raised.sort_unstable();
    assert_eq!(raised, [2, 3]);
    let kept = |id| get(&conn, OWNER, id).unwrap().is_some();
    assert_eq!(
        [kept("record"), kept("c1"), kept("c2"), kept("c3")],
        [true, false, true, true]
    );
    let state = |id| own_dm(&conn, OWNER, id).unwrap().unwrap().row.send_state;
    assert_eq!(state("c1"), SendState::Delivered);
    assert_eq!(state("c2"), SendState::Delivered);
    assert_ne!(state("c3"), SendState::Delivered);
    assert_ne!(state("record"), SendState::Delivered);
}

/// Review RV3 (f61572a0): an ack that finds no row still raises its echo
/// to delivered, and never lowers a read one.
#[test]
fn deliver_seq_raises_an_echo_with_no_row_and_never_lowers_one() {
    let conn = fixture::conn();
    for id in ["sent", "read"] {
        enqueue_with_echo(&conn, &row("dm:bob", id, 0), Some(&echo())).unwrap();
    }
    assert!(settle(&conn, OWNER, "sent", SendState::Sent).unwrap());
    assert!(settle(&conn, OWNER, "read", SendState::Read).unwrap());
    deliver_seq(&conn, OWNER, "dm:bob", "bob", 1).unwrap();
    deliver_seq(&conn, OWNER, "dm:bob", "bob", 2).unwrap();
    let state = |id| own_dm(&conn, OWNER, id).unwrap().unwrap().row.send_state;
    assert_eq!(state("sent"), SendState::Delivered);
    assert_eq!(state("read"), SendState::Read);
}
