//! `read_markers` (plan E3.1, D25): per channel or DM, the
//! device-independent position this identity has read up to — a channel
//! message's `lamport_ts`, a DM peer stream's `stream_seq`. A marker only
//! advances (`MAX`). Unread counts are derived here: the messages after the
//! marker, and of those the ones that mention us (`messages.mentions_me`).

use rusqlite::{params, Connection, OptionalExtension as _};

/// What a marker is a position in.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Scope {
    /// A community channel; the position is a message lamport.
    Channel {
        community_id: String,
        channel_id: String,
    },
    /// A 1:1 conversation; the position is the peer stream's seq.
    Dm { peer_key: String },
}

/// One channel's unread messages, of which `mentions` mention us.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Unread {
    pub messages: u32,
    pub mentions: u32,
}

fn to_i64(seq: u64) -> rusqlite::Result<i64> {
    i64::try_from(seq).map_err(|e| rusqlite::Error::ToSqlConversionFailure(e.into()))
}

fn to_u64(seq: i64) -> u64 {
    u64::try_from(seq).unwrap_or(0)
}

fn to_u32(count: i64) -> u32 {
    u32::try_from(count).unwrap_or(u32::MAX)
}

/// The marker on `scope` (0: nothing read yet).
///
/// # Errors
/// The query failed.
pub fn get(conn: &Connection, owner_key: &str, scope: &Scope) -> rusqlite::Result<u64> {
    let seq: Option<i64> = match scope {
        Scope::Channel {
            community_id,
            channel_id,
        } => conn
            .query_row(
                "SELECT last_read_seq FROM read_markers WHERE owner_key = ?1 \
                 AND scope_kind = 'channel' AND community_id = ?2 AND channel_id = ?3",
                params![owner_key, community_id, channel_id],
                |row| row.get(0),
            )
            .optional()?,
        Scope::Dm { peer_key } => conn
            .query_row(
                "SELECT last_read_seq FROM read_markers WHERE owner_key = ?1 \
                 AND scope_kind = 'dm' AND peer_key = ?2",
                params![owner_key, peer_key],
                |row| row.get(0),
            )
            .optional()?,
    };
    Ok(seq.map_or(0, to_u64))
}

/// Move the marker on `scope` up to `seq`; a lower or equal `seq` changes
/// nothing. A scope whose parent row (the community, the friend) is not
/// held here is skipped. Whether it moved.
///
/// # Errors
/// `seq` does not fit a row, or the write failed.
pub fn advance(
    conn: &Connection,
    owner_key: &str,
    scope: &Scope,
    seq: u64,
    now: i64,
) -> rusqlite::Result<bool> {
    let seq = to_i64(seq)?;
    let changed = match scope {
        Scope::Channel {
            community_id,
            channel_id,
        } => conn.execute(
            "INSERT INTO read_markers (owner_key, scope_kind, community_id, channel_id, \
             last_read_seq, updated_at) \
             SELECT ?1, 'channel', ?2, ?3, ?4, ?5 \
             WHERE EXISTS (SELECT 1 FROM communities WHERE owner_key = ?1 AND id = ?2) \
             ON CONFLICT(owner_key, community_id, channel_id) WHERE scope_kind = 'channel' \
             DO UPDATE SET last_read_seq = ?4, updated_at = ?5 \
             WHERE read_markers.last_read_seq < ?4",
            params![owner_key, community_id, channel_id, seq, now],
        )?,
        Scope::Dm { peer_key } => conn.execute(
            "INSERT INTO read_markers (owner_key, scope_kind, peer_key, last_read_seq, \
             updated_at) \
             SELECT ?1, 'dm', ?2, ?3, ?4 \
             WHERE EXISTS (SELECT 1 FROM friends WHERE owner_key = ?1 AND public_key = ?2) \
             ON CONFLICT(owner_key, peer_key) WHERE scope_kind = 'dm' \
             DO UPDATE SET last_read_seq = ?3, updated_at = ?4 \
             WHERE read_markers.last_read_seq < ?3",
            params![owner_key, peer_key, seq, now],
        )?,
    };
    Ok(changed > 0)
}

/// Every marker of `owner_key`, channels first, each kind in key order
/// (the personal-sync record's read state).
///
/// # Errors
/// The query failed.
pub fn all(conn: &Connection, owner_key: &str) -> rusqlite::Result<Vec<(Scope, u64)>> {
    let mut stmt = conn.prepare(
        "SELECT scope_kind, community_id, channel_id, peer_key, last_read_seq \
         FROM read_markers WHERE owner_key = ?1 \
         ORDER BY scope_kind, community_id, channel_id, peer_key",
    )?;
    let rows = stmt.query_map(params![owner_key], |row| {
        let kind: String = row.get(0)?;
        let scope = if kind == "channel" {
            Scope::Channel {
                community_id: row.get(1)?,
                channel_id: row.get(2)?,
            }
        } else {
            Scope::Dm {
                peer_key: row.get(3)?,
            }
        };
        Ok((scope, to_u64(row.get(4)?)))
    })?;
    rows.collect()
}

/// The lamport of the channel message `message_id` (the position a
/// `ChannelMarkRead` names), if it is stored.
///
/// # Errors
/// The query failed.
pub fn channel_position(
    conn: &Connection,
    owner_key: &str,
    channel_id: &str,
    message_id: &str,
) -> rusqlite::Result<Option<u64>> {
    let lamport: Option<Option<i64>> = conn
        .query_row(
            "SELECT lamport_ts FROM messages WHERE owner_key = ?1 \
             AND conversation_type = 'channel' AND conversation_id = ?2 AND message_id = ?3",
            params![owner_key, channel_id, message_id],
            |row| row.get(0),
        )
        .optional()?;
    Ok(lamport.map(|l| l.map_or(0, to_u64)))
}

/// The received messages of `channel_id` after `after` (a lamport). Our own
/// posts (`queued`, `sent`, `failed`) never count.
///
/// # Errors
/// The query failed.
pub fn channel_unread(
    conn: &Connection,
    owner_key: &str,
    channel_id: &str,
    after: u64,
) -> rusqlite::Result<Unread> {
    let (messages, mentions): (i64, i64) = conn.query_row(
        "SELECT COUNT(*), COALESCE(SUM(mentions_me), 0) FROM messages \
         WHERE owner_key = ?1 AND conversation_type = 'channel' AND conversation_id = ?2 \
         AND send_state = 'delivered' AND COALESCE(lamport_ts, 0) > ?3",
        params![owner_key, channel_id, to_i64(after)?],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    Ok(Unread {
        messages: to_u32(messages),
        mentions: to_u32(mentions),
    })
}

/// The messages `peer_key` sent us after `after` (a seq of their stream).
///
/// # Errors
/// The query failed.
pub fn dm_unread(
    conn: &Connection,
    owner_key: &str,
    peer_key: &str,
    after: u64,
) -> rusqlite::Result<u32> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM messages WHERE owner_key = ?1 AND conversation_type = 'dm' \
         AND conversation_id = ?2 AND sender_key = ?2 AND history_gap_through IS NULL \
         AND stream_seq > ?3",
        params![owner_key, peer_key, to_i64(after)?],
        |row| row.get(0),
    )?;
    Ok(to_u32(count))
}

/// The highest seq `peer_key` has sent us, the position a `DmMarkRead`
/// to the latest message names.
///
/// # Errors
/// The query failed.
pub fn dm_latest(conn: &Connection, owner_key: &str, peer_key: &str) -> rusqlite::Result<u64> {
    let seq: Option<i64> = conn.query_row(
        "SELECT MAX(stream_seq) FROM messages WHERE owner_key = ?1 AND conversation_type = 'dm' \
         AND conversation_id = ?2 AND sender_key = ?2",
        params![owner_key, peer_key],
        |row| row.get(0),
    )?;
    Ok(seq.map_or(0, to_u64))
}

/// How many channels and how many DMs have an unread message (the daemon
/// status snapshot).
///
/// # Errors
/// The query failed.
pub fn unread_scopes(conn: &Connection, owner_key: &str) -> rusqlite::Result<(usize, usize)> {
    let channels: i64 = conn.query_row(
        "SELECT COUNT(*) FROM channels c WHERE c.owner_key = ?1 AND EXISTS ( \
           SELECT 1 FROM messages m WHERE m.owner_key = ?1 AND m.conversation_type = 'channel' \
           AND m.conversation_id = c.id AND m.send_state = 'delivered' \
           AND COALESCE(m.lamport_ts, 0) > COALESCE((SELECT r.last_read_seq FROM read_markers r \
             WHERE r.owner_key = ?1 AND r.scope_kind = 'channel' \
             AND r.community_id = c.community_id AND r.channel_id = c.id), 0))",
        params![owner_key],
        |row| row.get(0),
    )?;
    let dms: i64 = conn.query_row(
        "SELECT COUNT(*) FROM friends f WHERE f.owner_key = ?1 AND EXISTS ( \
           SELECT 1 FROM messages m WHERE m.owner_key = ?1 AND m.conversation_type = 'dm' \
           AND m.conversation_id = f.public_key AND m.sender_key = f.public_key \
           AND m.history_gap_through IS NULL \
           AND m.stream_seq > COALESCE((SELECT r.last_read_seq FROM read_markers r \
             WHERE r.owner_key = ?1 AND r.scope_kind = 'dm' AND r.peer_key = f.public_key), 0))",
        params![owner_key],
        |row| row.get(0),
    )?;
    Ok((
        usize::try_from(channels).unwrap_or(0),
        usize::try_from(dms).unwrap_or(0),
    ))
}

#[cfg(test)]
#[path = "read_markers_tests.rs"]
mod tests;
