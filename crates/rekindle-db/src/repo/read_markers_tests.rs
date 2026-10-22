use rusqlite::params;

use super::*;
use crate::repo::fixture::{self, OWNER};

fn channel() -> Scope {
    Scope::Channel {
        community_id: "c".into(),
        channel_id: "ch".into(),
    }
}

fn dm() -> Scope {
    Scope::Dm {
        peer_key: "f".into(),
    }
}

fn seeded() -> rusqlite::Connection {
    let conn = fixture::conn();
    fixture::community(&conn, "c");
    conn.execute(
        "INSERT INTO friends (owner_key, public_key, added_at, friendship_state) \
         VALUES (?1, 'f', 0, 'accepted')",
        params![OWNER],
    )
    .unwrap();
    conn
}

fn channel_message(conn: &rusqlite::Connection, lamport: i64, state: &str, mentions: i64) {
    conn.execute(
        "INSERT INTO messages (owner_key, conversation_id, conversation_type, sender_key, body, \
         timestamp, lamport_ts, message_id, send_state, mentions_me) \
         VALUES (?1, 'ch', 'channel', 's', 'b', ?2, ?2, 'm' || ?2, ?3, ?4)",
        params![OWNER, lamport, state, mentions],
    )
    .unwrap();
}

fn dm_message(conn: &rusqlite::Connection, sender: &str, seq: i64) {
    conn.execute(
        "INSERT INTO messages (owner_key, conversation_id, conversation_type, sender_key, body, \
         timestamp, stream_seq) VALUES (?1, 'f', 'dm', ?2, 'b', ?3, ?3)",
        params![OWNER, sender, seq],
    )
    .unwrap();
}

#[test]
fn a_marker_only_advances() {
    let conn = seeded();
    for scope in [channel(), dm()] {
        assert_eq!(get(&conn, OWNER, &scope).unwrap(), 0);
        assert!(advance(&conn, OWNER, &scope, 5, 1).unwrap());
        assert!(!advance(&conn, OWNER, &scope, 4, 2).unwrap());
        assert!(!advance(&conn, OWNER, &scope, 5, 2).unwrap());
        assert_eq!(get(&conn, OWNER, &scope).unwrap(), 5);
        assert!(advance(&conn, OWNER, &scope, 9, 3).unwrap());
        assert_eq!(get(&conn, OWNER, &scope).unwrap(), 9);
    }
    assert_eq!(all(&conn, OWNER).unwrap(), vec![(channel(), 9), (dm(), 9)]);
}

#[test]
fn a_marker_without_its_parent_row_is_skipped() {
    let conn = fixture::conn();
    assert!(!advance(&conn, OWNER, &channel(), 3, 1).unwrap());
    assert!(!advance(&conn, OWNER, &dm(), 3, 1).unwrap());
    assert!(all(&conn, OWNER).unwrap().is_empty());
}

#[test]
fn counts_are_the_received_messages_after_the_marker() {
    let conn = seeded();
    channel_message(&conn, 1, "delivered", 0);
    channel_message(&conn, 2, "delivered", 1);
    channel_message(&conn, 3, "sent", 1);
    channel_message(&conn, 4, "delivered", 1);
    assert_eq!(
        channel_unread(&conn, OWNER, "ch", 0).unwrap(),
        Unread {
            messages: 3,
            mentions: 2
        }
    );
    assert_eq!(
        channel_unread(&conn, OWNER, "ch", 2).unwrap(),
        Unread {
            messages: 1,
            mentions: 1
        }
    );
    assert_eq!(channel_position(&conn, OWNER, "ch", "m2").unwrap(), Some(2));
    assert_eq!(channel_position(&conn, OWNER, "ch", "nope").unwrap(), None);

    dm_message(&conn, "f", 1);
    dm_message(&conn, "f", 2);
    dm_message(&conn, OWNER, 3);
    assert_eq!(dm_unread(&conn, OWNER, "f", 0).unwrap(), 2);
    assert_eq!(dm_unread(&conn, OWNER, "f", 1).unwrap(), 1);
    assert_eq!(dm_latest(&conn, OWNER, "f").unwrap(), 2);

    conn.execute(
        "INSERT INTO channels (owner_key, id, community_id, name, channel_type) \
         VALUES (?1, 'ch', 'c', 'general', 'text')",
        params![OWNER],
    )
    .unwrap();
    assert_eq!(unread_scopes(&conn, OWNER).unwrap(), (1, 1));
    advance(&conn, OWNER, &channel(), 4, 1).unwrap();
    advance(&conn, OWNER, &dm(), 2, 1).unwrap();
    assert_eq!(unread_scopes(&conn, OWNER).unwrap(), (0, 0));
}
