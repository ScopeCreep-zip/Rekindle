//! A received direct or group message's stored time (E2.B F3, threat
//! model: no field a sender claims decides on its own what the user sees).
//! The time the sender claims is clamped. It is never earlier than their
//! message with the next lower stream seq, never later than their message
//! with the next higher one, and never later than our receive time plus
//! the allowed clock skew. Within one sender, stream order is
//! authoritative: a false time cannot reorder their own messages, and a
//! far-future one cannot hold their message at the bottom.

use rusqlite::{params, Connection};

use super::messages::{insert_dm, NewDm, StoredDm};

/// The stored time of `dm` (a received message carrying its sender's
/// stream seq) received at `received_at` (ms), clamped as the module says.
///
/// # Errors
/// The query failed.
pub fn clamped_time(
    conn: &Connection,
    dm: &NewDm<'_>,
    received_at: i64,
    max_future_skew_ms: i64,
) -> rusqlite::Result<i64> {
    let mut at = dm
        .timestamp
        .min(received_at.saturating_add(max_future_skew_ms));
    let Some(seq) = dm.stream_seq else {
        return Ok(at);
    };
    let bound = |op: &str, agg: &str| -> rusqlite::Result<Option<i64>> {
        conn.query_row(
            &format!(
                "SELECT {agg}(timestamp) FROM messages WHERE owner_key = ?1 \
                 AND conversation_id = ?2 AND conversation_type = ?3 AND sender_key = ?4 \
                 AND stream_seq {op} ?5"
            ),
            params![
                dm.owner_key,
                dm.peer_key,
                dm.conversation_type.as_str(),
                dm.sender_key,
                seq
            ],
            |r| r.get(0),
        )
    };
    if let Some(later) = bound(">", "MIN")? {
        at = at.min(later);
    }
    if let Some(earlier) = bound("<", "MAX")? {
        at = at.max(earlier);
    }
    Ok(at)
}

/// Store the received `dm` at its [`clamped_time`]; its row, and the time
/// stored.
///
/// # Errors
/// The read or the write failed.
pub fn insert_received(
    conn: &Connection,
    dm: &NewDm<'_>,
    received_at: i64,
    max_future_skew_ms: i64,
) -> rusqlite::Result<(StoredDm, i64)> {
    let timestamp = clamped_time(conn, dm, received_at, max_future_skew_ms)?;
    let stored = insert_dm(conn, &NewDm { timestamp, ..*dm })?;
    Ok((stored, timestamp))
}
