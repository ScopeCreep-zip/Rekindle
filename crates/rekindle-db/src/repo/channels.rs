//! `channels`: a community's channels as this identity knows them, with
//! each channel's message record key.

use rusqlite::{params, Connection};

/// A channel row to write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredChannel<'a> {
    pub id: &'a str,
    pub name: &'a str,
    /// One of the schema's `channel_type` values (`text`, `voice`, …).
    pub channel_type: &'a str,
    pub category_id: Option<&'a str>,
    pub topic: &'a str,
    pub slowmode_seconds: u32,
    pub nsfw: bool,
    pub message_record_key: Option<&'a str>,
    pub mek_generation: u64,
    pub parent_voice_channel_id: Option<&'a str>,
}

fn insert_with(
    conn: &Connection,
    verb: &str,
    owner_key: &str,
    community_id: &str,
    channel: &StoredChannel<'_>,
) -> rusqlite::Result<()> {
    conn.execute(
        &format!(
            "{verb} INTO channels (owner_key, id, community_id, name, channel_type, category_id, \
             topic, slowmode_seconds, nsfw, message_record_key, mek_generation, \
             parent_voice_channel_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)"
        ),
        params![
            owner_key,
            channel.id,
            community_id,
            channel.name,
            channel.channel_type,
            channel.category_id,
            channel.topic,
            i64::from(channel.slowmode_seconds),
            i32::from(channel.nsfw),
            channel.message_record_key,
            channel.mek_generation.cast_signed(),
            channel.parent_voice_channel_id,
        ],
    )?;
    Ok(())
}

/// Insert a channel.
///
/// # Errors
/// The write failed (the channel already exists).
pub fn insert(
    conn: &Connection,
    owner_key: &str,
    community_id: &str,
    channel: &StoredChannel<'_>,
) -> rusqlite::Result<()> {
    insert_with(conn, "INSERT", owner_key, community_id, channel)
}

/// Insert a channel unless it is already there.
///
/// # Errors
/// The write failed.
pub fn insert_if_absent(
    conn: &Connection,
    owner_key: &str,
    community_id: &str,
    channel: &StoredChannel<'_>,
) -> rusqlite::Result<()> {
    insert_with(conn, "INSERT OR IGNORE", owner_key, community_id, channel)
}

/// Delete a channel.
///
/// # Errors
/// The write failed.
pub fn delete(
    conn: &Connection,
    owner_key: &str,
    community_id: &str,
    channel_id: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "DELETE FROM channels WHERE owner_key = ?1 AND community_id = ?2 AND id = ?3",
        params![owner_key, community_id, channel_id],
    )?;
    Ok(())
}

/// Set a channel's message record key when it has none: the first key
/// learned stands (a channel's record never changes).
///
/// # Errors
/// The write failed.
pub fn set_record_key_if_absent(
    conn: &Connection,
    owner_key: &str,
    community_id: &str,
    channel_id: &str,
    record_key: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE channels SET message_record_key = ?4 \
         WHERE owner_key = ?1 AND community_id = ?2 AND id = ?3 AND message_record_key IS NULL",
        params![owner_key, community_id, channel_id, record_key],
    )?;
    Ok(())
}

/// Every channel of the community that has a message record:
/// `(channel id, record key)`.
///
/// # Errors
/// The query failed.
pub fn record_keys(
    conn: &Connection,
    owner_key: &str,
    community_id: &str,
) -> rusqlite::Result<Vec<(String, String)>> {
    let mut stmt = conn.prepare(
        "SELECT id, message_record_key FROM channels \
         WHERE owner_key = ?1 AND community_id = ?2 AND message_record_key IS NOT NULL \
         ORDER BY id",
    )?;
    let rows = stmt
        .query_map(params![owner_key, community_id], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })?
        .collect();
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::communities::{self, NewCommunity};
    use crate::repo::fixture::{self, OWNER};

    fn channel<'a>(id: &'a str, key: Option<&'a str>) -> StoredChannel<'a> {
        StoredChannel {
            id,
            name: "general",
            channel_type: "text",
            category_id: None,
            topic: "",
            slowmode_seconds: 0,
            nsfw: false,
            message_record_key: key,
            mek_generation: 1,
            parent_voice_channel_id: None,
        }
    }

    #[test]
    fn record_keys_are_set_once_and_listed() {
        let conn = fixture::conn();
        communities::insert(
            &conn,
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
        insert(&conn, OWNER, "c1", &channel("a", Some("ka"))).unwrap();
        insert(&conn, OWNER, "c1", &channel("b", None)).unwrap();
        insert_if_absent(&conn, OWNER, "c1", &channel("a", Some("other"))).unwrap();
        assert!(insert(&conn, OWNER, "c1", &channel("a", None)).is_err());
        set_record_key_if_absent(&conn, OWNER, "c1", "b", "kb").unwrap();
        set_record_key_if_absent(&conn, OWNER, "c1", "a", "replaced").unwrap();
        assert_eq!(
            record_keys(&conn, OWNER, "c1").unwrap(),
            vec![("a".into(), "ka".into()), ("b".into(), "kb".into())]
        );
        delete(&conn, OWNER, "c1", "a").unwrap();
        assert_eq!(record_keys(&conn, OWNER, "c1").unwrap().len(), 1);
    }
}

/// Store our own send sequence of a channel.
///
/// # Errors
/// The write failed.
pub fn set_my_sequence(
    conn: &Connection,
    owner_key: &str,
    community_id: &str,
    channel_id: &str,
    sequence: i64,
) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE channels SET my_sequence = ?1 WHERE owner_key = ?2 AND community_id = ?3 AND id = ?4",
        params![sequence, owner_key, community_id, channel_id],
    )?;
    Ok(())
}

/// Store the time of our last send in a channel, which its slowmode
/// counts from.
///
/// # Errors
/// The write failed.
pub fn set_last_send(
    conn: &Connection,
    owner_key: &str,
    community_id: &str,
    channel_id: &str,
    last_send_ms: i64,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO channel_slowmode_state (owner_key, community_id, channel_id, last_send_ms) \
         VALUES (?1, ?2, ?3, ?4) \
         ON CONFLICT(owner_key, community_id, channel_id) DO UPDATE SET \
           last_send_ms = excluded.last_send_ms",
        params![owner_key, community_id, channel_id, last_send_ms],
    )?;
    Ok(())
}
