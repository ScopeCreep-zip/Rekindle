//! `messages`: the delivery state of our own sent messages (plan D1,
//! C7.14). The inserts are still the desktop's (`message_repo`).

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
