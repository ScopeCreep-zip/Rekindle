//! A community's scheduled events and RSVPs (plan E3.1 E4): the rows the
//! mesh's `EventCreated` / `EventUpdated` / `EventRsvpChanged` messages and
//! our own commands store. Merged governance is the durable record; these
//! rows carry what it does not (`max_attendees`, `created_at`) and what
//! arrived over gossip before the next merge.

use rekindle_types::event::EventInfo;
use rusqlite::{params, Connection};

/// Store `event`, replacing a stored row with the same id.
///
/// # Errors
/// The insert failed.
pub fn upsert(
    conn: &Connection,
    owner_key: &str,
    community_id: &str,
    event: &EventInfo,
) -> rusqlite::Result<()> {
    let recurrence = event
        .recurrence
        .as_ref()
        .map(serde_json::to_string)
        .transpose()
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
    let location = event
        .location
        .as_ref()
        .map(serde_json::to_string)
        .transpose()
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
    conn.execute(
        "INSERT OR REPLACE INTO community_events \
         (owner_key, community_id, id, title, description, creator_pseudonym, \
          start_time, end_time, channel_id, max_attendees, created_at, status, \
          cover_image_ref, recurrence_json, location_json) \
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)",
        params![
            owner_key,
            community_id,
            event.id,
            event.title,
            event.description,
            event.creator_pseudonym,
            event.start_time.cast_signed(),
            event.end_time.map(u64::cast_signed),
            event.channel_id,
            event.max_attendees,
            event.created_at.cast_signed(),
            event.status,
            event.cover_image_ref,
            recurrence,
            location,
        ],
    )?;
    Ok(())
}

/// The community's stored events, by start time then id; their RSVP lists
/// are empty (the community store holds them).
///
/// # Errors
/// The read failed.
pub fn list(
    conn: &Connection,
    owner_key: &str,
    community_id: &str,
) -> rusqlite::Result<Vec<EventInfo>> {
    let mut stmt = conn.prepare(
        "SELECT id, title, description, creator_pseudonym, start_time, end_time, \
                channel_id, max_attendees, created_at, status, \
                cover_image_ref, recurrence_json, location_json \
         FROM community_events \
         WHERE owner_key = ?1 AND community_id = ?2 \
         ORDER BY start_time ASC, id ASC",
    )?;
    let rows = stmt.query_map(params![owner_key, community_id], |row| {
        Ok(EventInfo {
            id: row.get(0)?,
            title: row.get(1)?,
            description: row.get(2)?,
            creator_pseudonym: row.get(3)?,
            start_time: row.get::<_, i64>(4)?.cast_unsigned(),
            end_time: row.get::<_, Option<i64>>(5)?.map(i64::cast_unsigned),
            channel_id: row.get(6)?,
            max_attendees: row.get::<_, Option<u32>>(7)?,
            created_at: row.get::<_, i64>(8)?.cast_unsigned(),
            status: row.get(9)?,
            rsvps: Vec::new(),
            cover_image_ref: row.get(10)?,
            recurrence: crate::row::get_json_opt(row, "recurrence_json")?,
            location: crate::row::get_json_opt(row, "location_json")?,
        })
    })?;
    rows.collect()
}

/// Drop the event `event_id` and its RSVPs.
///
/// # Errors
/// A delete failed.
pub fn remove(
    conn: &Connection,
    owner_key: &str,
    community_id: &str,
    event_id: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "DELETE FROM community_events WHERE owner_key = ?1 AND community_id = ?2 AND id = ?3",
        params![owner_key, community_id, event_id],
    )?;
    conn.execute(
        "DELETE FROM event_rsvps WHERE owner_key = ?1 AND community_id = ?2 AND event_id = ?3",
        params![owner_key, community_id, event_id],
    )?;
    Ok(())
}

/// Store one member's answer to `event_id`.
///
/// # Errors
/// The insert failed.
pub fn upsert_rsvp(
    conn: &Connection,
    owner_key: &str,
    community_id: &str,
    event_id: &str,
    pseudonym_key: &str,
    status: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO event_rsvps \
         (owner_key, community_id, event_id, pseudonym_key, status) \
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![owner_key, community_id, event_id, pseudonym_key, status],
    )?;
    Ok(())
}
