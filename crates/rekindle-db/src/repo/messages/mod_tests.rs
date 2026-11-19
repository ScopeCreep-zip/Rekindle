use super::*;
use crate::repo::channels::{self, StoredChannel};
use crate::repo::communities::{self, NewCommunity};
use crate::repo::fixture::{self, OWNER};
use crate::repo::history_gap;

fn seed(conn: &Connection) {
    communities::insert(
        conn,
        OWNER,
        &NewCommunity {
            id: "c1",
            name: "C",
            joined_at: 1,
            dht_owner_keypair: None,
            my_pseudonym_key: "p",
            mek_generation: 1,
            member_registry_key: None,
            my_subkey_index: None,
        },
    )
    .unwrap();
    channels::insert(
        conn,
        OWNER,
        "c1",
        &StoredChannel {
            id: "ch1",
            name: "general",
            channel_type: "text",
            category_id: None,
            topic: "",
            slowmode_seconds: 0,
            nsfw: false,
            mek_generation: 1,
            parent_voice_channel_id: None,
        },
    )
    .unwrap();
    conn.execute(
        "INSERT INTO messages (owner_key, conversation_id, conversation_type, sender_key, body, \
         timestamp, message_id, send_state) VALUES (?1, 'ch1', 'channel', 'p', 'hi', 1, 'm1', 'queued')",
        [OWNER],
    )
    .unwrap();
}

/// A message `body` with client id `id`.
fn dm<'a>(
    peer: &'a str,
    sender: &'a str,
    ts: i64,
    (body, id): (&'a str, &'a str),
    reply_to: Option<&'a str>,
) -> NewDm<'a> {
    NewDm {
        owner_key: OWNER,
        peer_key: peer,
        conversation_type: rekindle_types::message::ConversationType::Dm,
        expire_seconds: 0,
        expire_at: None,
        sender_key: sender,
        body,
        timestamp: ts,
        is_read: sender == OWNER,
        message_id: id,
        reply_to,
        stream_seq: None,
        send_state: if sender == OWNER {
            SendState::Queued
        } else {
            SendState::Delivered
        },
    }
}

#[test]
fn a_reply_points_at_its_parent_and_a_replayed_message_is_stored_once() {
    let conn = fixture::conn();
    let parent = insert_dm(&conn, &dm("bob", "bob", 1, ("m1", "id1"), None))
        .unwrap()
        .id;
    let reply = insert_dm(&conn, &dm("bob", OWNER, 2, ("m2", "id2"), Some("id1")))
        .unwrap()
        .id;
    let orphan = insert_dm(&conn, &dm("bob", "bob", 3, ("m3", "id3"), Some("gone")))
        .unwrap()
        .id;
    let reply_to = |id: i64| -> Option<i64> {
        conn.query_row(
            "SELECT reply_to_id FROM messages WHERE id = ?1",
            [id],
            |r| r.get(0),
        )
        .unwrap()
    };
    assert_eq!(reply_to(reply), Some(parent));
    assert_eq!(reply_to(orphan), None);
    assert_eq!(
        insert_dm(&conn, &dm("bob", "bob", 1, ("m1", "id1"), None)).unwrap(),
        StoredDm {
            id: parent,
            inserted: false
        }
    );
    assert_eq!(dm_history(&conn, OWNER, "bob", 10, None).unwrap().len(), 3);
}

/// Audit D-2: messages are not deduplicated by time. Two inbound and
/// two outbound DMs in the same millisecond are four rows, and a peer's
/// message reusing our client id is a row of its own.
#[test]
fn messages_in_one_millisecond_are_all_stored() {
    let conn = fixture::conn();
    for (sender, id) in [
        ("bob", "in1"),
        ("bob", "in2"),
        (OWNER, "out1"),
        (OWNER, "out2"),
    ] {
        assert!(
            insert_dm(&conn, &dm("bob", sender, 7, ("same ms", id), None))
                .unwrap()
                .inserted
        );
    }
    let ours = insert_dm(&conn, &dm("bob", OWNER, 8, ("mine", "dup"), None)).unwrap();
    let theirs = insert_dm(&conn, &dm("bob", "bob", 8, ("theirs", "dup"), None)).unwrap();
    assert!(theirs.inserted);
    assert_ne!(ours.id, theirs.id);
    let history = dm_history(&conn, OWNER, "bob", 10, None).unwrap();
    assert_eq!(history.len(), 6);
    assert_eq!(history.iter().filter(|m| m.timestamp == 7).count(), 4);
    assert_eq!(
        history.iter().find(|m| m.id == ours.id).unwrap().body,
        "mine"
    );
}

#[test]
fn dm_history_pages_back_and_the_inbox_shows_each_conversation_once() {
    let conn = fixture::conn();
    for (peer, sender, ts, text) in [
        ("bob", "bob", 1, ("m1", "id1")),
        ("bob", OWNER, 2, ("m2", "id2")),
        ("bob", "bob", 3, ("m3", "id3")),
        ("carol", "carol", 5, ("hey", "id5")),
    ] {
        insert_dm(&conn, &dm(peer, sender, ts, text, None)).unwrap();
    }
    let newest: Vec<i64> = dm_history(&conn, OWNER, "bob", 2, None)
        .unwrap()
        .iter()
        .map(|m| m.timestamp)
        .collect();
    assert_eq!(newest, [2, 3]);
    let older = dm_history(&conn, OWNER, "bob", 2, Some(2)).unwrap();
    assert_eq!(older.len(), 1);
    assert_eq!(older[0].body, "m1");
    assert!(!older[0].is_read);
    mark_dm_read(&conn, OWNER, "bob").unwrap();
    assert!(dm_history(&conn, OWNER, "bob", 10, None)
        .unwrap()
        .iter()
        .all(|m| m.is_read));
    let inbox: Vec<(String, String)> = dm_inbox(&conn, OWNER, 10)
        .unwrap()
        .into_iter()
        .map(|m| (m.peer_key, m.body))
        .collect();
    assert_eq!(
        inbox,
        [("carol".into(), "hey".into()), ("bob".into(), "m3".into())]
    );
}

/// D23: a history gap is a local row of the conversation, stored once
/// per `(peer, through_seq)`, read in history and kept off the inbox.
#[test]
fn a_history_gap_is_stored_once_and_read_back_in_history() {
    let conn = fixture::conn();
    insert_dm(&conn, &dm("bob", "bob", 1, ("m1", "id1"), None)).unwrap();
    let bob = ("bob", rekindle_types::message::ConversationType::Dm);
    let gap = history_gap::insert(&conn, OWNER, bob, "bob", 4, 2).unwrap();
    assert_eq!(
        history_gap::insert(&conn, OWNER, bob, "bob", 4, 3).unwrap(),
        gap
    );
    assert_ne!(
        history_gap::insert(&conn, OWNER, bob, "bob", 9, 3).unwrap(),
        gap
    );
    assert!(history_gap::insert(&conn, OWNER, bob, "bob", 0, 3).is_err());
    let history = dm_history(&conn, OWNER, "bob", 10, None).unwrap();
    assert_eq!(history.len(), 3);
    let marker = history.iter().find(|m| m.id == gap).unwrap();
    assert_eq!(marker.history_gap_through, Some(4));
    assert_eq!(
        (marker.sender_key.as_str(), marker.body.as_str()),
        ("bob", "")
    );
    assert!(marker.is_read && marker.send_state == SendState::Delivered);
    assert_eq!(history[0].history_gap_through, None);
    let inbox = dm_inbox(&conn, OWNER, 10).unwrap();
    assert_eq!(inbox.len(), 1);
    assert_eq!(inbox[0].body, "m1");
}

#[test]
fn a_channel_post_row_projects_its_settle_onto_the_post() {
    let conn = fixture::conn();
    seed(&conn);
    let row = super::super::outbox::NewRow {
        owner_key: OWNER,
        stream: "channel_post:rk:4",
        client_id: "m1",
        kind: "channel_post",
        payload: b"{}",
        next_at: 0,
        deadline: i64::MAX,
    };
    let seq = super::super::outbox::enqueue_for_message(&conn, &row, "m1").unwrap();
    let rows = super::super::outbox::stream_rows(&conn, OWNER, "channel_post:rk:4").unwrap();
    assert_eq!(rows.len(), 1);
    // The echo is at the row's per-stream seq (D22), as its readers
    // store the entry.
    let placed: (Option<i64>, i64) = conn
        .query_row(
            "SELECT stream_seq, lamport_ts FROM messages WHERE message_id = 'm1'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(placed, (Some(seq), seq));
    super::super::outbox::settle(&conn, OWNER, "m1", SendState::Sent).unwrap();
    assert!(
        super::super::outbox::stream_rows(&conn, OWNER, "channel_post:rk:4")
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        send_state(&conn, OWNER, "m1").unwrap(),
        Some(SendState::Sent)
    );
    assert!(conn
        .execute(
            "UPDATE messages SET send_state = 'lost' WHERE message_id = 'm1'",
            [],
        )
        .is_err());
}

#[test]
fn a_message_names_its_conversation_and_type() {
    use rekindle_types::message::ConversationType;
    let conn = fixture::conn();
    insert_dm(&conn, &dm("bob", "bob", 1, ("m1", "id1"), None)).unwrap();
    assert_eq!(
        super::conversation_of(&conn, OWNER, "id1").unwrap(),
        Some(("bob".to_string(), ConversationType::Dm))
    );
    conn.execute(
        "INSERT INTO messages (owner_key, conversation_id, conversation_type, sender_key, \
         body, timestamp, message_id) VALUES (?1, 'g', 'group_dm', 's', 'b', 0, 'id2')",
        [OWNER],
    )
    .unwrap();
    assert_eq!(
        super::conversation_of(&conn, OWNER, "id2").unwrap(),
        Some(("g".to_string(), ConversationType::GroupDm))
    );
    assert_eq!(super::conversation_of(&conn, OWNER, "none").unwrap(), None);
}
