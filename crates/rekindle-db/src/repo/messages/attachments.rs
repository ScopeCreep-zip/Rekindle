//! The attachment records stored on channel message rows
//! (`messages.attachment_json`, plan E3.1 E3d): the Host reads a record to
//! name, play or reveal an attachment, and stores the place a download was
//! saved to.

use rusqlite::{params, Connection};

/// Every channel row of `channel_id` that carries an attachment record:
/// `(message_id, attachment_json)`, in row order.
///
/// # Errors
/// The query failed.
pub fn channel_records(
    conn: &Connection,
    owner_key: &str,
    channel_id: &str,
) -> rusqlite::Result<Vec<(Option<String>, String)>> {
    let mut stmt = conn.prepare(
        "SELECT message_id, attachment_json FROM messages \
         WHERE owner_key = ?1 AND conversation_id = ?2 AND conversation_type = 'channel' \
         AND attachment_json IS NOT NULL ORDER BY id",
    )?;
    let rows = stmt.query_map(params![owner_key, channel_id], |row| {
        Ok((
            crate::row::get::<Option<String>>(row, "message_id")?,
            crate::row::get::<String>(row, "attachment_json")?,
        ))
    })?;
    rows.collect()
}

/// Replace the attachment record of the row `message_id` of `channel_id`.
///
/// # Errors
/// The update failed.
pub fn set_record(
    conn: &Connection,
    owner_key: &str,
    channel_id: &str,
    message_id: &str,
    attachment_json: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE messages SET attachment_json = ?1 \
         WHERE owner_key = ?2 AND conversation_id = ?3 AND message_id = ?4",
        params![attachment_json, owner_key, channel_id, message_id],
    )?;
    Ok(())
}
