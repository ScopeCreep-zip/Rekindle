//! Channel persistence helpers.
//!
//! Pure `rusqlite` functions that encapsulate SQL for the `channels` table.
//! Callers wrap these in `db_call` or `db_fire` as appropriate.

use crate::state::ChannelInfo;

/// Insert a single channel.
pub fn insert_channel(
    conn: &rusqlite::Connection,
    owner_key: &str,
    channel: &ChannelInfo,
    community_id: &str,
) -> Result<(), rusqlite::Error> {
    conn.execute(
        "INSERT INTO channels (owner_key, id, community_id, name, channel_type) VALUES (?, ?, ?, ?, ?)",
        rusqlite::params![owner_key, channel.id, community_id, channel.name, channel.channel_type],
    )?;
    Ok(())
}

/// Insert a channel, ignoring conflicts (e.g. duplicate primary key).
pub fn upsert_channel(
    conn: &rusqlite::Connection,
    owner_key: &str,
    channel: &ChannelInfo,
    community_id: &str,
) -> Result<(), rusqlite::Error> {
    conn.execute(
        "INSERT OR IGNORE INTO channels (owner_key, id, community_id, name, channel_type) VALUES (?, ?, ?, ?, ?)",
        rusqlite::params![owner_key, channel.id, community_id, channel.name, channel.channel_type],
    )?;
    Ok(())
}

/// Replace all channels for a community: DELETE existing + batch INSERT OR IGNORE.
pub fn replace_channels(
    conn: &rusqlite::Connection,
    owner_key: &str,
    community_id: &str,
    channels: &[ChannelInfo],
) -> Result<(), rusqlite::Error> {
    conn.execute(
        "DELETE FROM channels WHERE owner_key = ? AND community_id = ?",
        rusqlite::params![owner_key, community_id],
    )?;
    for ch in channels {
        upsert_channel(conn, owner_key, ch, community_id)?;
    }
    Ok(())
}

/// Delete a single channel by ID.
pub fn delete_channel(
    conn: &rusqlite::Connection,
    owner_key: &str,
    channel_id: &str,
    community_id: &str,
) -> Result<(), rusqlite::Error> {
    conn.execute(
        "DELETE FROM channels WHERE owner_key = ? AND id = ? AND community_id = ?",
        rusqlite::params![owner_key, channel_id, community_id],
    )?;
    Ok(())
}
