//! The voice session analytics log (`voice_session_events`): one row per
//! join or leave of a community voice channel.

use rusqlite::Connection;

/// One voice session event of ours.
pub struct VoiceSessionEventRow<'a> {
    pub owner_key: &'a str,
    pub community_id: &'a str,
    pub channel_id: &'a str,
    pub member_pseudonym: &'a str,
    /// `"join"` or `"leave"`.
    pub event_type: &'a str,
    pub occurred_at_ms: i64,
}

/// Record `event`.
///
/// # Errors
/// The insert failed.
pub fn record(conn: &Connection, event: &VoiceSessionEventRow<'_>) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO voice_session_events \
         (owner_key, community_id, channel_id, member_pseudonym, event_type, occurred_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![
            event.owner_key,
            event.community_id,
            event.channel_id,
            event.member_pseudonym,
            event.event_type,
            event.occurred_at_ms
        ],
    )?;
    Ok(())
}
