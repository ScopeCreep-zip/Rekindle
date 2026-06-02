//! Conn-level audit-entry persistence.
//!
//! Pure `rusqlite` functions used inside `db_call`/`db_fire` closures. Caller
//! owns chain-state advancement (the `AuditChain` in `AppState::audit_chain`);
//! these are pure I/O.

use rekindle_audit::{AuditEntry, AuditRecord};
use rusqlite::{params, Connection};

/// Insert one audit entry. Caller owns chain-state advancement (i.e. the
/// `AuditChain` instance in `AppState::audit_chain`); this is pure I/O.
pub fn insert_entry(
    conn: &Connection,
    owner_key: &str,
    entry: &AuditEntry,
) -> Result<(), rusqlite::Error> {
    let payload_json = serde_json::to_string(&entry.record)
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
    let cursor_i64 = entry.cursor.cast_signed();
    conn.execute(
        "INSERT INTO audit_entries (owner_key, cursor, prev_mac, mac, payload_json) \
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            owner_key,
            cursor_i64,
            &entry.prev_mac[..],
            &entry.mac[..],
            payload_json,
        ],
    )?;
    Ok(())
}

/// Load every entry for `owner_key` in cursor order.
pub fn load_all(conn: &Connection, owner_key: &str) -> Result<Vec<AuditEntry>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT cursor, prev_mac, mac, payload_json FROM audit_entries \
         WHERE owner_key = ?1 ORDER BY cursor ASC",
    )?;
    let rows = stmt.query_map(params![owner_key], row_to_entry)?;
    rows.collect()
}

/// Load entries with `cursor > since_cursor` for export.
pub fn load_since(
    conn: &Connection,
    owner_key: &str,
    since_cursor: u64,
) -> Result<Vec<AuditEntry>, rusqlite::Error> {
    let since_i64 = since_cursor.cast_signed();
    let mut stmt = conn.prepare(
        "SELECT cursor, prev_mac, mac, payload_json FROM audit_entries \
         WHERE owner_key = ?1 AND cursor > ?2 ORDER BY cursor ASC",
    )?;
    let rows = stmt.query_map(params![owner_key, since_i64], row_to_entry)?;
    rows.collect()
}

/// Latest (cursor, mac) for `owner_key`, or `(0, [0u8; 32])` if empty.
/// Used to initialize the in-memory `AuditChain` on vault unlock.
pub fn load_tail(conn: &Connection, owner_key: &str) -> Result<(u64, [u8; 32]), rusqlite::Error> {
    use rusqlite::OptionalExtension as _;
    let row: Option<(i64, Vec<u8>)> = conn
        .query_row(
            "SELECT cursor, mac FROM audit_entries \
             WHERE owner_key = ?1 ORDER BY cursor DESC LIMIT 1",
            params![owner_key],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let Some((cursor_i64, mac_bytes)) = row else {
        return Ok((0, [0u8; 32]));
    };
    if mac_bytes.len() != 32 || cursor_i64 < 0 {
        // Truncated/corrupt — surface during verify rather than panic here.
        return Ok((0, [0u8; 32]));
    }
    let mut mac = [0u8; 32];
    mac.copy_from_slice(&mac_bytes);
    let cursor = cursor_i64.cast_unsigned();
    Ok((cursor, mac))
}

fn row_to_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<AuditEntry> {
    let cursor_i64: i64 = row.get(0)?;
    let prev_mac_bytes: Vec<u8> = row.get(1)?;
    let mac_bytes: Vec<u8> = row.get(2)?;
    let payload_json: String = row.get(3)?;
    let cursor = cursor_i64.cast_unsigned();
    let record: AuditRecord = serde_json::from_str(&payload_json).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    })?;
    let mut prev_mac = [0u8; 32];
    let mut mac = [0u8; 32];
    if prev_mac_bytes.len() != 32 || mac_bytes.len() != 32 {
        return Err(rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Blob,
            format!(
                "audit_entries row at cursor {cursor} has malformed MAC blobs (prev_mac={}B, mac={}B)",
                prev_mac_bytes.len(),
                mac_bytes.len()
            )
            .into(),
        ));
    }
    prev_mac.copy_from_slice(&prev_mac_bytes);
    mac.copy_from_slice(&mac_bytes);
    Ok(AuditEntry {
        cursor,
        prev_mac,
        mac,
        record,
    })
}
