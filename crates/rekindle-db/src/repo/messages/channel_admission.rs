//! Reader-side channel post placeholders.

use rekindle_types::dto::channel_message::PostAdmission;
use rusqlite::{params, Connection};

/// A pending or refused post in a channel history page.
#[derive(Debug)]
pub struct Placeholder {
    pub sender: String,
    pub message_id: String,
    pub timestamp: i64,
    pub status: PostAdmission,
}

/// The keys and metadata of a post as its verified chain carries them.
pub struct Post<'a> {
    pub owner: &'a str,
    pub community: &'a str,
    pub stream: &'a str,
    pub sender: &'a str,
    pub message: &'a str,
    pub record: &'a str,
    pub timestamp: i64,
}

/// Store a verdict or clear the placeholder when admission finishes.
///
/// # Errors
/// The write or status serialization failed.
pub fn set(
    conn: &Connection,
    post: Post<'_>,
    status: Option<PostAdmission>,
) -> rusqlite::Result<bool> {
    let changed = if let Some(status) = status {
        let json = serde_json::to_string(&status)
            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
        conn.execute(
            "INSERT INTO channel_post_admission
                (owner_key, community_id, stream_id, sender, message_id, member_record, timestamp, status)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT (owner_key, community_id, stream_id, sender, message_id)
             DO UPDATE SET status = excluded.status WHERE status != excluded.status",
            params![post.owner, post.community, post.stream, post.sender, post.message,
                post.record, post.timestamp, json],
        )?
    } else {
        conn.execute(
            "DELETE FROM channel_post_admission
             WHERE owner_key = ?1 AND community_id = ?2 AND stream_id = ?3 AND sender = ?4 AND message_id = ?5",
            params![post.owner, post.community, post.stream, post.sender, post.message],
        )?
    };
    Ok(changed != 0)
}

/// Read the newest placeholders in the requested history page.
///
/// # Errors
/// The query failed or a stored status did not decode.
pub fn page(
    conn: &Connection,
    owner: &str,
    community: &str,
    stream: &str,
    before: Option<i64>,
    limit: u32,
) -> rusqlite::Result<Vec<Placeholder>> {
    let mut query = conn.prepare(
        "SELECT sender, message_id, timestamp, status FROM channel_post_admission
         WHERE owner_key = ?1 AND community_id = ?2 AND stream_id = ?3
           AND (?4 IS NULL OR timestamp < ?4)
         ORDER BY timestamp DESC, message_id DESC LIMIT ?5",
    )?;
    let rows = query.query_map(params![owner, community, stream, before, limit], |row| {
        let text: String = row.get(3)?;
        let status = serde_json::from_str(&text).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(3, rusqlite::types::Type::Text, Box::new(e))
        })?;
        Ok(Placeholder {
            sender: row.get(0)?,
            message_id: row.get(1)?,
            timestamp: row.get(2)?,
            status,
        })
    })?;
    rows.collect()
}

/// Member records with an unresolved cut. Their cursor still covers the entry.
///
/// # Errors
/// The query failed.
pub fn pending_records(
    conn: &Connection,
    owner: &str,
    community: &str,
) -> rusqlite::Result<Vec<String>> {
    let mut query = conn.prepare(
        "SELECT DISTINCT member_record FROM channel_post_admission
         WHERE owner_key = ?1 AND community_id = ?2 AND json_extract(status, '$.status') = 'pending'
         ORDER BY member_record",
    )?;
    let rows = query.query_map(params![owner, community], |row| row.get(0))?;
    rows.collect()
}
