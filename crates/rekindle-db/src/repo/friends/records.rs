//! The receive path's view of `friends` (`FriendStore`): one
//! [`FriendRecord`] per friend.

use rekindle_types::friend_store::FriendRecord;
use rusqlite::{params, params_from_iter, Connection, OptionalExtension as _};

const RECORD_COLS: &str = "public_key, dht_record_key, mailbox_dht_key, \
    current_device_id, display_name, added_at, friendship_state";

fn to_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<FriendRecord> {
    Ok(FriendRecord {
        pubkey_hex: row.get(0)?,
        inbox_record_key: row.get::<_, Option<String>>(1)?.unwrap_or_default(),
        mailbox_record_key: row.get::<_, Option<String>>(2)?.unwrap_or_default(),
        current_device_id: row.get(3)?,
        display_name: row.get::<_, Option<String>>(4)?.unwrap_or_default(),
        // `added_at` is milliseconds; the record carries microseconds.
        added_at_us: crate::row::get_u64(row, 5)?.saturating_mul(1_000),
        status: crate::row::get_enum(row, "friendship_state")?,
    })
}

/// The friend with `public_key`.
///
/// # Errors
/// The query failed.
pub fn record(
    conn: &Connection,
    owner_key: &str,
    public_key: &str,
) -> rusqlite::Result<Option<FriendRecord>> {
    conn.query_row(
        &format!("SELECT {RECORD_COLS} FROM friends WHERE owner_key = ?1 AND public_key = ?2"),
        params![owner_key, public_key],
        to_record,
    )
    .optional()
}

/// The friend whose profile (inbox) record is `record_key`.
///
/// # Errors
/// The query failed.
pub fn record_by_profile_key(
    conn: &Connection,
    owner_key: &str,
    record_key: &str,
) -> rusqlite::Result<Option<FriendRecord>> {
    conn.query_row(
        &format!("SELECT {RECORD_COLS} FROM friends WHERE owner_key = ?1 AND dht_record_key = ?2"),
        params![owner_key, record_key],
        to_record,
    )
    .optional()
}

/// The friends among `public_keys`.
///
/// # Errors
/// The query failed.
pub fn records(
    conn: &Connection,
    owner_key: &str,
    public_keys: &[String],
) -> rusqlite::Result<Vec<FriendRecord>> {
    if public_keys.is_empty() {
        return Ok(Vec::new());
    }
    let placeholders = (2..=public_keys.len() + 1)
        .map(|i| format!("?{i}"))
        .collect::<Vec<_>>()
        .join(", ");
    let mut stmt = conn.prepare(&format!(
        "SELECT {RECORD_COLS} FROM friends WHERE owner_key = ?1 AND public_key IN ({placeholders})"
    ))?;
    let rows = stmt
        .query_map(
            params_from_iter(
                std::iter::once(owner_key).chain(public_keys.iter().map(String::as_str)),
            ),
            to_record,
        )?
        .collect();
    rows
}

/// Every accepted friend.
///
/// # Errors
/// The query failed.
pub fn active_records(conn: &Connection, owner_key: &str) -> rusqlite::Result<Vec<FriendRecord>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {RECORD_COLS} FROM friends WHERE owner_key = ?1 AND friendship_state = 'accepted'"
    ))?;
    let rows = stmt.query_map(params![owner_key], to_record)?.collect();
    rows
}
