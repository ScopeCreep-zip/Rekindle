//! `messages`: direct messages (plan E2.1) and the delivery state of our
//! own sent messages (plan D1, C7.14). The channel inserts are still the
//! desktop's (`message_repo`, until E3.1).

use rekindle_types::message::SendState;
use rusqlite::{params, Connection};

/// A queued message waiting on a held write's record slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeldMessage {
    pub record_key: String,
    pub subkey: u32,
    pub community_id: String,
    pub channel_id: String,
    pub message_id: String,
}

/// Set a message's delivery state. `held` is the record slot a `Queued`
/// message waits on; any other state clears it.
///
/// # Errors
/// The write failed.
pub fn set_send_state(
    conn: &Connection,
    owner_key: &str,
    message_id: &str,
    state: SendState,
    held: Option<(&str, u32)>,
) -> rusqlite::Result<()> {
    let (record, subkey) = match (state, held) {
        (SendState::Queued, Some((record, subkey))) => (Some(record), Some(subkey)),
        _ => (None, None),
    };
    conn.execute(
        "UPDATE messages SET send_state = ?1, held_record_key = ?2, held_subkey = ?3 \
         WHERE owner_key = ?4 AND message_id = ?5",
        params![state.as_str(), record, subkey, owner_key, message_id],
    )?;
    Ok(())
}

/// Every queued channel message waiting on a held write, with its
/// community (the next login re-registers them).
///
/// # Errors
/// The query failed.
pub fn held_channel_messages(
    conn: &Connection,
    owner_key: &str,
) -> rusqlite::Result<Vec<HeldMessage>> {
    let mut stmt = conn.prepare(
        "SELECT m.held_record_key, m.held_subkey, c.community_id, m.conversation_id, m.message_id \
         FROM messages m JOIN channels c ON c.owner_key = m.owner_key AND c.id = m.conversation_id \
         WHERE m.owner_key = ?1 AND m.conversation_type = 'channel' AND m.send_state = 'queued' \
         AND m.held_record_key IS NOT NULL AND m.held_subkey IS NOT NULL \
         AND m.message_id IS NOT NULL",
    )?;
    let rows = stmt
        .query_map(params![owner_key], |row| {
            Ok(HeldMessage {
                record_key: row.get(0)?,
                subkey: row.get(1)?,
                community_id: row.get(2)?,
                channel_id: row.get(3)?,
                message_id: row.get(4)?,
            })
        })?
        .collect();
    rows
}

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
}

/// A direct message to store (plan E2.3): `message_id` is its client id,
/// minted by its author, and `reply_to` the parent's.
#[derive(Debug, Clone, Copy)]
pub struct NewDm<'a> {
    pub owner_key: &'a str,
    pub peer_key: &'a str,
    pub sender_key: &'a str,
    pub body: &'a str,
    pub timestamp: i64,
    /// Ours is read; a received one is unread until the window marks it.
    pub is_read: bool,
    pub message_id: &'a str,
    /// Resolved to the parent's row; `NULL` when the parent is not held.
    pub reply_to: Option<&'a str>,
}

/// A stored direct message: its row, and whether this call stored it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StoredDm {
    pub id: i64,
    /// `false` when its client id was already held (a replay of an
    /// applied message, D24): nothing was written.
    pub inserted: bool,
}

/// Store a direct message. A message whose client id is already held is
/// not stored again: the held row is returned, not inserted.
///
/// # Errors
/// The write failed.
pub fn insert_dm(conn: &Connection, dm: &NewDm<'_>) -> rusqlite::Result<StoredDm> {
    let inserted = conn.execute(
        "INSERT INTO messages (owner_key, conversation_id, conversation_type, sender_key, body, \
         timestamp, is_read, message_id, reply_to_id) \
         VALUES (?1, ?2, 'dm', ?3, ?4, ?5, ?6, ?7, \
                 (SELECT id FROM messages WHERE owner_key = ?1 AND conversation_id = ?2 \
                  AND message_id = ?8)) \
         ON CONFLICT(owner_key, conversation_id, message_id) WHERE message_id IS NOT NULL \
         DO NOTHING",
        params![
            dm.owner_key,
            dm.peer_key,
            dm.sender_key,
            dm.body,
            dm.timestamp,
            dm.is_read,
            dm.message_id,
            dm.reply_to
        ],
    )? > 0;
    let id = conn.query_row(
        "SELECT id FROM messages WHERE owner_key = ?1 AND conversation_id = ?2 AND message_id = ?3",
        params![dm.owner_key, dm.peer_key, dm.message_id],
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
    })
}

const DM_COLS: &str =
    "id, conversation_id, sender_key, body, timestamp, is_read, send_state, message_id";

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
                   ORDER BY n.timestamp DESC, n.id DESC LIMIT 1) \
         ORDER BY timestamp DESC LIMIT ?2"
    ))?;
    let rows = stmt.query_map(params![owner_key, limit], dm_row)?.collect();
    rows
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
            sender_key: sender,
            body,
            timestamp: ts,
            is_read: sender == OWNER,
            message_id: id,
            reply_to,
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

    #[test]
    fn a_held_post_is_listed_until_it_settles() {
        let conn = fixture::conn();
        seed(&conn);
        set_send_state(&conn, OWNER, "m1", SendState::Queued, Some(("rk", 4))).unwrap();
        let held = held_channel_messages(&conn, OWNER).unwrap();
        assert_eq!(
            held,
            vec![HeldMessage {
                record_key: "rk".into(),
                subkey: 4,
                community_id: "c1".into(),
                channel_id: "ch1".into(),
                message_id: "m1".into(),
            }]
        );
        set_send_state(&conn, OWNER, "m1", SendState::Sent, Some(("rk", 4))).unwrap();
        assert!(held_channel_messages(&conn, OWNER).unwrap().is_empty());
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
}
