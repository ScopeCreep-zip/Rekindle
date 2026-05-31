//! Architecture §32 Phase 7 Week 23 — "storage usage per community".
//!
//! SQLite doesn't expose per-row byte cost directly, but `length(...)`
//! across the textual columns plus a small fixed per-row overhead is a
//! solid approximation. We sum the relevant tables for the given
//! community: messages, thread messages, channel pins, channel-read
//! state, voice session events, member-leave log, and the static
//! community/channel/role/category metadata.
//!
//! Numbers are advisory — the UI displays them as "approximately N
//! KB", not as a billing-grade exact byte count.

use rekindle_types::analytics::StorageUsage;
use rusqlite::Connection;

/// Approximate per-row overhead — index entries, hidden ROWID, small
/// integer columns we don't bother summing. Tuned against
/// `dbstat`-measured rows in the SQLite shell.
const ROW_OVERHEAD_BYTES: u64 = 48;

pub fn compute(conn: &Connection, owner_key: &str, community_id: &str) -> StorageUsage {
    let message_bytes = sum_table_bytes(
        conn,
        "messages",
        "owner_key = ?1 AND conversation_id IN (\
              SELECT id FROM channels WHERE owner_key = ?1 AND community_id = ?2)",
        &["body", "attachment_json"],
        owner_key,
        community_id,
    );
    let thread_message_bytes = sum_table_bytes(
        conn,
        "thread_messages",
        "owner_key = ?1 AND community_id = ?2",
        &["body"],
        owner_key,
        community_id,
    );
    let channel_pin_bytes = sum_table_bytes(
        conn,
        "channel_pins",
        "owner_key = ?1 AND community_id = ?2",
        &["message_id", "pinned_by"],
        owner_key,
        community_id,
    );
    let read_state_bytes = sum_table_bytes(
        conn,
        "channel_read_state",
        "owner_key = ?1 AND community_id = ?2",
        &["channel_id"],
        owner_key,
        community_id,
    );
    let voice_event_bytes = sum_table_bytes(
        conn,
        "voice_session_events",
        "owner_key = ?1 AND community_id = ?2",
        &["channel_id", "member_pseudonym", "event_type"],
        owner_key,
        community_id,
    );
    let member_leave_bytes = sum_table_bytes(
        conn,
        "community_member_leaves",
        "owner_key = ?1 AND community_id = ?2",
        &["pseudonym_key"],
        owner_key,
        community_id,
    );
    let metadata_bytes = community_metadata_bytes(conn, owner_key, community_id);

    let total_bytes = message_bytes
        .saturating_add(thread_message_bytes)
        .saturating_add(channel_pin_bytes)
        .saturating_add(read_state_bytes)
        .saturating_add(voice_event_bytes)
        .saturating_add(member_leave_bytes)
        .saturating_add(metadata_bytes);

    StorageUsage {
        total_bytes,
        message_bytes,
        thread_message_bytes,
        channel_pin_bytes,
        read_state_bytes,
        voice_event_bytes,
        member_leave_bytes,
        metadata_bytes,
    }
}

fn sum_table_bytes(
    conn: &Connection,
    table: &str,
    where_clause: &str,
    text_columns: &[&str],
    owner_key: &str,
    community_id: &str,
) -> u64 {
    let length_terms: String = text_columns
        .iter()
        .map(|col| format!("COALESCE(LENGTH({col}), 0)"))
        .collect::<Vec<_>>()
        .join(" + ");
    // SUM over LENGTH columns + per-row overhead; safe to interpolate
    // `table` and `text_columns` because they're hard-coded constants
    // from this module, not user input.
    let sql = format!(
        "SELECT COALESCE(SUM({length_terms}), 0) + COUNT(*) * ?3 \
           FROM {table} \
          WHERE {where_clause}"
    );
    let bytes: i64 = conn
        .query_row(
            &sql,
            rusqlite::params![owner_key, community_id, ROW_OVERHEAD_BYTES],
            |r| r.get(0),
        )
        .unwrap_or(0);
    u64::try_from(bytes).unwrap_or(0)
}

fn community_metadata_bytes(conn: &Connection, owner_key: &str, community_id: &str) -> u64 {
    // Single community row: name + description + JSON config blobs.
    let community_row: i64 = conn
        .query_row(
            "SELECT COALESCE(LENGTH(name), 0) + COALESCE(LENGTH(description), 0) \
                  + COALESCE(LENGTH(my_role_ids), 0) + COALESCE(LENGTH(dht_record_key), 0) \
                  + ?3 \
               FROM communities WHERE owner_key = ?1 AND id = ?2",
            rusqlite::params![owner_key, community_id, ROW_OVERHEAD_BYTES],
            |r| r.get(0),
        )
        .unwrap_or(0);
    let channels_row: i64 = conn
        .query_row(
            "SELECT COALESCE(SUM(LENGTH(name) + COALESCE(LENGTH(topic), 0)), 0) \
                  + COUNT(*) * ?3 \
               FROM channels WHERE owner_key = ?1 AND community_id = ?2",
            rusqlite::params![owner_key, community_id, ROW_OVERHEAD_BYTES],
            |r| r.get(0),
        )
        .unwrap_or(0);
    let roles_row: i64 = conn
        .query_row(
            "SELECT COALESCE(SUM(LENGTH(name)), 0) + COUNT(*) * ?3 \
               FROM community_roles WHERE owner_key = ?1 AND community_id = ?2",
            rusqlite::params![owner_key, community_id, ROW_OVERHEAD_BYTES],
            |r| r.get(0),
        )
        .unwrap_or(0);
    u64::try_from(
        community_row
            .saturating_add(channels_row)
            .saturating_add(roles_row),
    )
    .unwrap_or(0)
}
