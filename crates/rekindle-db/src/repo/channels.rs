//! `channels`: a community's channels as this identity knows them (a
//! channel has no record of its own, plan E3.3 slice 8).

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
             topic, slowmode_seconds, nsfw, mek_generation, \
             parent_voice_channel_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)"
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::communities::{self, NewCommunity};
    use crate::repo::fixture::{self, OWNER};

    fn channel(id: &str) -> StoredChannel<'_> {
        StoredChannel {
            id,
            name: "general",
            channel_type: "text",
            category_id: None,
            topic: "",
            slowmode_seconds: 0,
            nsfw: false,
            mek_generation: 1,
            parent_voice_channel_id: None,
        }
    }

    #[test]
    fn a_channel_is_inserted_once_and_deleted() {
        let conn = fixture::conn();
        communities::insert(
            &conn,
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
        insert(&conn, OWNER, "c1", &channel("a")).unwrap();
        insert_if_absent(&conn, OWNER, "c1", &channel("a")).unwrap();
        assert!(insert(&conn, OWNER, "c1", &channel("a")).is_err());
        delete(&conn, OWNER, "c1", "a").unwrap();
        insert(&conn, OWNER, "c1", &channel("a")).unwrap();
    }
}
