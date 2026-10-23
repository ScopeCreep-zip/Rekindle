//! The peer's receipts on our rows (D22): an early `DmAck` or `ReadUpTo`
//! is shown on the echo at once and kept on a row not landed.

use rusqlite::{params, Connection};

use rekindle_types::message::SendState;

/// The peer read our stream to `peer_key` (`stream`) through `up_to`: every
/// echo at or below it not yet `read` becomes `read`, and the rows still
/// waiting there settle (the peer holds them), in one transaction. The
/// echoes raised, as `(stream_seq, client_id)`.
///
/// # Errors
/// The write failed; nothing changed.
pub fn read_through(
    conn: &Connection,
    owner_key: &str,
    stream: &str,
    peer_key: &str,
    up_to: i64,
) -> rusqlite::Result<Vec<(i64, String)>> {
    let tx = conn.unchecked_transaction()?;
    // A covered row whose append has not landed (`queued`: the peer read
    // the hint first) is marked read, not settled: it settles on its
    // landing, as an early `DmAck` does (live A/B finding 42 follow-up).
    // Its echo shows `read` now (D22: the state comes from the peer's
    // marker; the landing only lets the row go).
    let marked = {
        let mut stmt = tx.prepare(
            "UPDATE outbox SET peer_ack = 'read' WHERE owner_key = ?1 AND stream = ?2 \
             AND stream_seq <= ?3 AND state = 'queued' \
             AND (peer_ack IS NULL OR peer_ack != 'read') RETURNING stream_seq, client_id",
        )?;
        let rows = stmt.query_map(params![owner_key, stream, up_to], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })?;
        rows.collect::<rusqlite::Result<Vec<(i64, String)>>>()?
    };
    for (_, client_id) in &marked {
        project_ack(&tx, owner_key, client_id, SendState::Read)?;
    }
    let mut raised = {
        let mut stmt = tx.prepare(
            "UPDATE messages SET send_state = 'read' WHERE owner_key = ?1 \
             AND conversation_id = ?2 AND conversation_type = 'dm' AND sender_key = ?1 \
             AND stream_seq IS NOT NULL AND stream_seq <= ?3 AND message_id IS NOT NULL \
             AND send_state != 'read' AND NOT EXISTS (SELECT 1 FROM outbox o \
             WHERE o.owner_key = ?1 AND o.stream = ?4 AND o.stream_seq = messages.stream_seq \
             AND o.state = 'queued') RETURNING stream_seq, message_id",
        )?;
        let rows = stmt.query_map(params![owner_key, peer_key, up_to, stream], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })?;
        rows.collect::<rusqlite::Result<Vec<(i64, String)>>>()?
    };
    raised.extend(marked);
    raised.sort();
    tx.execute(
        "DELETE FROM outbox WHERE owner_key = ?1 AND stream = ?2 AND stream_seq <= ?3 \
         AND state != 'queued'",
        params![owner_key, stream, up_to],
    )?;
    tx.commit()?;
    Ok(raised)
}

/// Record the peer's `ack` (`delivered` or `read`; the higher one stays) on
/// `client_id` while its send has not landed (`queued`, live A/B finding
/// 42). Returns whether it was recorded; `false` when the row is past
/// `queued` or gone, and the ack settles it instead.
///
/// # Errors
/// The write failed; nothing changed.
pub fn note_ack(
    conn: &Connection,
    owner_key: &str,
    client_id: &str,
    ack: SendState,
) -> rusqlite::Result<bool> {
    let tx = conn.unchecked_transaction()?;
    let noted = tx.execute(
        "UPDATE outbox SET peer_ack = CASE WHEN peer_ack = 'read' THEN 'read' ELSE ?3 END \
         WHERE owner_key = ?1 AND client_id = ?2 AND state = 'queued'",
        params![owner_key, client_id, ack.as_str()],
    )? > 0;
    if noted {
        project_ack(&tx, owner_key, client_id, ack)?;
    }
    tx.commit()?;
    Ok(noted)
}

/// Show the peer's `ack` on `client_id`'s echo at once (D22: `delivered`
/// and `read` come from the peer), never lowering a `read`.
fn project_ack(
    conn: &Connection,
    owner_key: &str,
    client_id: &str,
    ack: SendState,
) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE messages SET send_state = ?3 WHERE owner_key = ?1 AND message_id = ?2 \
         AND sender_key = ?1 AND send_state != 'read'",
        params![owner_key, client_id, ack.as_str()],
    )?;
    Ok(())
}
