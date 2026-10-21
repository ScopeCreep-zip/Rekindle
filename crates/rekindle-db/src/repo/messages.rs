//! `messages`: direct messages (plan E2.1) and the delivery state of our
//! own sent messages (plan D1, C7.14), which the outbox's rows project
//! (`repo::outbox`, plan E2.5). The channel inserts are still the
//! desktop's (`message_repo`, until E3.1).

use rekindle_types::message::SendState;
use rusqlite::{params, Connection};

/// A message's delivery state, if it exists.
///
/// # Errors
/// The query failed or the stored state is not in the vocabulary.
pub fn send_state(
    conn: &Connection,
    owner_key: &str,
    message_id: &str,
) -> rusqlite::Result<Option<SendState>> {
    use rusqlite::OptionalExtension as _;
    conn.query_row(
        "SELECT send_state FROM messages WHERE owner_key = ?1 AND message_id = ?2",
        params![owner_key, message_id],
        |row| {
            let stored: String = row.get(0)?;
            SendState::parse(&stored).ok_or_else(|| {
                rusqlite::Error::FromSqlConversionFailure(
                    0,
                    rusqlite::types::Type::Text,
                    format!("unknown send state {stored}").into(),
                )
            })
        },
    )
    .optional()
}

/// A direct message, as a chat window shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DmRow {
    pub id: i64,
    /// The client id its author minted (plan E2.3).
    pub message_id: Option<String>,
    pub peer_key: String,
    pub sender_key: String,
    pub body: String,
    pub timestamp: i64,
    pub is_read: bool,
    pub send_state: SendState,
    /// A history-gap marker's last retired seq (plan E2.5, D23); `None`
    /// for a message.
    pub history_gap_through: Option<u64>,
}

/// A direct message to store (plan E2.3): `message_id` is its client id,
/// minted by its author, and `reply_to` the parent's.
#[derive(Debug, Clone, Copy)]
pub struct NewDm<'a> {
    pub owner_key: &'a str,
    /// The conversation: the peer, or a group's id (32 hex, plan E2.6).
    pub peer_key: &'a str,
    /// `Dm`, or `GroupDm` for a group's message.
    pub conversation_type: rekindle_types::message::ConversationType,
    pub sender_key: &'a str,
    pub body: &'a str,
    pub timestamp: i64,
    /// Ours is read; a received one is unread until the window marks it.
    pub is_read: bool,
    pub message_id: &'a str,
    /// Resolved to the parent's row; `NULL` when the parent is not held.
    pub reply_to: Option<&'a str>,
    /// Ours is `queued` until the send's outcome (`sent` or `failed`)
    /// replaces it; a received one is `delivered` (plan audit D-3).
    pub send_state: SendState,
    /// Its place in its DM stream (ours: our outbox seq; received: the
    /// sender's), which a `ReadUpTo` names (plan E2.5).
    pub stream_seq: Option<i64>,
}

/// A stored direct message: its row, and whether this call stored it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StoredDm {
    pub id: i64,
    /// `false` when its client id was already held (a replay of an
    /// applied message, D24): nothing was written.
    pub inserted: bool,
}

/// Store a direct message. A message whose client id is already held from
/// the same sender is not stored again: the held row is returned, not
/// inserted. Messages are told apart by `(sender, client id)`, never by
/// time, so two in one millisecond are both stored, and a peer reusing
/// our client id stores a row of its own. A reply points at the first
/// held row with the parent's client id.
///
/// # Errors
/// The write failed.
pub fn insert_dm(conn: &Connection, dm: &NewDm<'_>) -> rusqlite::Result<StoredDm> {
    let inserted = conn.execute(
        "INSERT INTO messages (owner_key, conversation_id, conversation_type, sender_key, body, \
         timestamp, is_read, message_id, reply_to_id, send_state, stream_seq) \
         VALUES (?1, ?2, ?11, ?3, ?4, ?5, ?6, ?7, \
                 (SELECT id FROM messages WHERE owner_key = ?1 AND conversation_id = ?2 \
                  AND message_id = ?8 ORDER BY id LIMIT 1), ?9, ?10) \
         ON CONFLICT(owner_key, conversation_id, sender_key, message_id) \
         WHERE message_id IS NOT NULL DO NOTHING",
        params![
            dm.owner_key,
            dm.peer_key,
            dm.sender_key,
            dm.body,
            dm.timestamp,
            dm.is_read,
            dm.message_id,
            dm.reply_to,
            dm.send_state.as_str(),
            dm.stream_seq,
            dm.conversation_type.as_str()
        ],
    )? > 0;
    let id = conn.query_row(
        "SELECT id FROM messages WHERE owner_key = ?1 AND conversation_id = ?2 \
         AND sender_key = ?3 AND message_id = ?4",
        params![dm.owner_key, dm.peer_key, dm.sender_key, dm.message_id],
        |row| row.get(0),
    )?;
    Ok(StoredDm { id, inserted })
}

fn dm_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<DmRow> {
    let stored: String = row.get(6)?;
    Ok(DmRow {
        id: row.get(0)?,
        peer_key: row.get(1)?,
        sender_key: row.get(2)?,
        body: row.get(3)?,
        timestamp: row.get(4)?,
        is_read: row.get(5)?,
        message_id: row.get(7)?,
        send_state: SendState::parse(&stored).ok_or_else(|| {
            rusqlite::Error::FromSqlConversionFailure(
                6,
                rusqlite::types::Type::Text,
                format!("unknown send state {stored}").into(),
            )
        })?,
        history_gap_through: row
            .get::<_, Option<i64>>(8)?
            .map(|seq| {
                u64::try_from(seq).map_err(|e| {
                    rusqlite::Error::FromSqlConversionFailure(
                        8,
                        rusqlite::types::Type::Integer,
                        Box::new(e),
                    )
                })
            })
            .transpose()?,
    })
}

const DM_COLS: &str = "id, conversation_id, sender_key, body, timestamp, is_read, send_state, \
                       message_id, history_gap_through";

/// Store the local marker of a DM history gap (plan E2.5, D23): `peer`'s
/// chain entries through `through_seq` were retired before we fetched
/// them. It is never sent: an empty body from the peer, read and
/// `delivered`. A second marker for the same `(peer, through_seq)` is not
/// stored; the held row's id is returned.
///
/// # Errors
/// `through_seq` is not a stored seq (0 or above `i64::MAX`), or the write
/// failed.
pub fn insert_history_gap(
    conn: &Connection,
    owner_key: &str,
    peer: &str,
    through_seq: u64,
    timestamp: i64,
) -> rusqlite::Result<i64> {
    let through = i64::try_from(through_seq)
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
    conn.execute(
        "INSERT INTO messages (owner_key, conversation_id, conversation_type, sender_key, body, \
         timestamp, is_read, send_state, history_gap_through) \
         SELECT ?1, ?2, 'dm', ?2, '', ?3, 1, 'delivered', ?4 \
         WHERE NOT EXISTS (SELECT 1 FROM messages WHERE owner_key = ?1 AND conversation_id = ?2 \
                           AND conversation_type = 'dm' AND history_gap_through = ?4)",
        params![owner_key, peer, timestamp, through],
    )?;
    conn.query_row(
        "SELECT id FROM messages WHERE owner_key = ?1 AND conversation_id = ?2 \
         AND conversation_type = 'dm' AND history_gap_through = ?3",
        params![owner_key, peer, through],
        |row| row.get(0),
    )
}

/// The newest `limit` messages with `peer_key` before `before` (a
/// timestamp, ms; all when `None`), oldest first.
///
/// # Errors
/// The query failed.
pub fn dm_history(
    conn: &Connection,
    owner_key: &str,
    peer_key: &str,
    limit: u32,
    before: Option<i64>,
) -> rusqlite::Result<Vec<DmRow>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {DM_COLS} FROM messages WHERE owner_key = ?1 AND conversation_id = ?2 \
         AND conversation_type = 'dm' AND timestamp < ?3 ORDER BY timestamp DESC, id DESC LIMIT ?4"
    ))?;
    let mut rows = stmt
        .query_map(
            params![owner_key, peer_key, before.unwrap_or(i64::MAX), limit],
            dm_row,
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    rows.reverse();
    Ok(rows)
}

/// One of our own direct messages, by its client id, with its parent's
/// client id when it is a reply: what a resend frames again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnDm {
    pub row: DmRow,
    pub reply_to: Option<String>,
}

/// Our direct message whose client id is `message_id`; `None` when we sent
/// none with that id.
///
/// # Errors
/// The query failed.
pub fn own_dm(
    conn: &Connection,
    owner_key: &str,
    message_id: &str,
) -> rusqlite::Result<Option<OwnDm>> {
    use rusqlite::OptionalExtension as _;
    conn.query_row(
        "SELECT m.id, m.conversation_id, m.sender_key, m.body, m.timestamp, m.is_read, \
         m.send_state, m.message_id, m.history_gap_through, p.message_id FROM messages m \
         LEFT JOIN messages p ON p.id = m.reply_to_id \
         WHERE m.owner_key = ?1 AND m.sender_key = ?1 AND m.message_id = ?2 \
         AND m.conversation_type = 'dm'",
        params![owner_key, message_id],
        |row| {
            Ok(OwnDm {
                row: dm_row(row)?,
                reply_to: row.get(9)?,
            })
        },
    )
    .optional()
}

/// The stream seq of `peer_key`'s message `message_id` (its client id),
/// as it arrived; `None` when no such received message is held or it
/// carried none (plan E2.5: the window reports the newest one it rendered).
///
/// # Errors
/// The read failed.
pub fn received_stream_seq(
    conn: &Connection,
    owner_key: &str,
    peer_key: &str,
    message_id: &str,
) -> rusqlite::Result<Option<i64>> {
    use rusqlite::OptionalExtension as _;
    conn.query_row(
        "SELECT stream_seq FROM messages WHERE owner_key = ?1 AND conversation_id = ?2 \
         AND conversation_type = 'dm' AND sender_key = ?2 AND message_id = ?3",
        params![owner_key, peer_key, message_id],
        |row| row.get(0),
    )
    .optional()
    .map(Option::flatten)
}

/// Mark every message from `peer_key` read.
///
/// # Errors
/// The write failed.
pub fn mark_dm_read(conn: &Connection, owner_key: &str, peer_key: &str) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE messages SET is_read = 1 WHERE owner_key = ?1 AND conversation_id = ?2 \
         AND conversation_type = 'dm' AND is_read = 0",
        params![owner_key, peer_key],
    )?;
    Ok(())
}

/// The latest message of each direct conversation, newest first, at most
/// `limit`.
///
/// # Errors
/// The query failed.
pub fn dm_inbox(conn: &Connection, owner_key: &str, limit: u32) -> rusqlite::Result<Vec<DmRow>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {DM_COLS} FROM messages m WHERE owner_key = ?1 AND conversation_type = 'dm' \
         AND id = (SELECT id FROM messages n WHERE n.owner_key = m.owner_key \
                   AND n.conversation_id = m.conversation_id AND n.conversation_type = 'dm' \
                   AND n.history_gap_through IS NULL ORDER BY n.timestamp DESC, n.id DESC LIMIT 1) \
         ORDER BY timestamp DESC LIMIT ?2"
    ))?;
    let rows = stmt.query_map(params![owner_key, limit], dm_row)?.collect();
    rows
}

/// The conversation a message of ours or a peer's belongs to, named by its
/// client id: `(conversation id, type)` of the first row holding it.
///
/// # Errors
/// The query failed or the stored type is not in the vocabulary.
pub fn conversation_of(
    conn: &Connection,
    owner_key: &str,
    message_id: &str,
) -> rusqlite::Result<Option<(String, rekindle_types::message::ConversationType)>> {
    use rusqlite::OptionalExtension as _;
    conn.query_row(
        "SELECT conversation_id, conversation_type FROM messages \
         WHERE owner_key = ?1 AND message_id = ?2 ORDER BY id LIMIT 1",
        params![owner_key, message_id],
        |row| {
            Ok((
                crate::row::get(row, "conversation_id")?,
                crate::row::get_enum(row, "conversation_type")?,
            ))
        },
    )
    .optional()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::channels::{self, StoredChannel};
    use crate::repo::communities::{self, NewCommunity};
    use crate::repo::fixture::{self, OWNER};

    fn seed(conn: &Connection) {
        communities::insert(
            conn,
            OWNER,
            &NewCommunity {
                id: "c1",
                name: "C",
                my_role_ids: &[0],
                joined_at: 1,
                dht_owner_keypair: None,
                my_pseudonym_key: "p",
                mek_generation: 1,
                member_registry_key: None,
                my_subkey_index: None,
                governance_key: Some("c1"),
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
                message_record_key: None,
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
        let gap = insert_history_gap(&conn, OWNER, "bob", 4, 2).unwrap();
        assert_eq!(insert_history_gap(&conn, OWNER, "bob", 4, 3).unwrap(), gap);
        assert_ne!(insert_history_gap(&conn, OWNER, "bob", 9, 3).unwrap(), gap);
        assert!(insert_history_gap(&conn, OWNER, "bob", 0, 3).is_err());
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
        super::super::outbox::enqueue_for_message(&conn, &row, "m1").unwrap();
        let rows = super::super::outbox::stream_rows(&conn, OWNER, "channel_post:rk:4").unwrap();
        assert_eq!(rows.len(), 1);
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
}
