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
    ack_range(
        conn,
        owner_key,
        (stream, peer_key),
        (0, up_to),
        SendState::Read,
    )
}

/// The peer holds entry `stream_seq` of our stream to `peer_key` (a seq a
/// `DmAck` names): its echo below `delivered` becomes `delivered`, whether
/// or not its row is still here (an ack that finds no row still raises the
/// stored state), and its row settles if it landed, in one transaction.
/// The echoes raised.
///
/// # Errors
/// The write failed; nothing changed.
pub fn deliver_seq(
    conn: &Connection,
    owner_key: &str,
    stream: &str,
    peer_key: &str,
    stream_seq: i64,
) -> rusqlite::Result<Vec<(i64, String)>> {
    let tx = conn.unchecked_transaction()?;
    let raised = ack_range_in(
        &tx,
        owner_key,
        (stream, peer_key),
        (stream_seq, stream_seq),
        SendState::Delivered,
    )?;
    tx.commit()?;
    Ok(raised)
}

/// The peer applied our chain to `peer_key` through chain seq `through` (a
/// cumulative `DmAck`): every entry it carried (`chain_entries`, chain seq
/// to stream seq; a `StreamRecord` row is not on the chain and is not
/// covered) is delivered as [`deliver_seq`] does, in one transaction.
///
/// # Errors
/// The write failed; nothing changed.
pub fn deliver_chain_through(
    conn: &Connection,
    owner_key: &str,
    stream: &str,
    peer_key: &str,
    through: i64,
) -> rusqlite::Result<Vec<(i64, String)>> {
    let tx = conn.unchecked_transaction()?;
    // Only the entries with something left to raise or settle.
    let seqs: Vec<i64> = {
        let mut stmt = tx.prepare(
            "SELECT c.stream_seq FROM chain_entries c WHERE c.owner_key = ?1 AND c.peer = ?2 \
             AND c.chain_seq <= ?3 AND (EXISTS (SELECT 1 FROM outbox o WHERE o.owner_key = ?1 \
             AND o.stream = ?4 AND o.stream_seq = c.stream_seq) OR EXISTS (SELECT 1 FROM \
             messages m WHERE m.owner_key = ?1 AND m.conversation_id = ?2 \
             AND m.conversation_type = 'dm' AND m.sender_key = ?1 \
             AND m.stream_seq = c.stream_seq AND m.send_state NOT IN ('delivered', 'read'))) \
             ORDER BY c.stream_seq",
        )?;
        let rows = stmt.query_map(params![owner_key, peer_key, through, stream], |r| r.get(0))?;
        rows.collect::<rusqlite::Result<Vec<i64>>>()?
    };
    let mut raised = Vec::new();
    for seq in seqs {
        raised.extend(ack_range_in(
            &tx,
            owner_key,
            (stream, peer_key),
            (seq, seq),
            SendState::Delivered,
        )?);
    }
    tx.commit()?;
    Ok(raised)
}

fn ack_range(
    conn: &Connection,
    owner_key: &str,
    stream: (&str, &str),
    range: (i64, i64),
    ack: SendState,
) -> rusqlite::Result<Vec<(i64, String)>> {
    let tx = conn.unchecked_transaction()?;
    let raised = ack_range_in(&tx, owner_key, stream, range, ack)?;
    tx.commit()?;
    Ok(raised)
}

/// The peer's `ack` (`delivered` or `read`) of entries `lo..=hi`, inside
/// the caller's transaction.
fn ack_range_in(
    tx: &Connection,
    owner_key: &str,
    (stream, peer_key): (&str, &str),
    (lo, hi): (i64, i64),
    ack: SendState,
) -> rusqlite::Result<Vec<(i64, String)>> {
    // A covered row whose append has not landed (`queued`: the peer read
    // the hint first) keeps the ack, not settled: it settles on its
    // landing, as an early `DmAck` does (live A/B finding 42 follow-up).
    // Its echo shows the ack now (D22: the state comes from the peer; the
    // landing only lets the row go). `read` is never lowered.
    let marked = {
        let mut stmt = tx.prepare(
            "UPDATE outbox SET peer_ack = ?5 WHERE owner_key = ?1 AND stream = ?2 \
             AND stream_seq BETWEEN ?3 AND ?4 AND state = 'queued' \
             AND (peer_ack IS NULL OR (peer_ack != 'read' AND peer_ack != ?5)) \
             RETURNING stream_seq, client_id",
        )?;
        let rows = stmt.query_map(params![owner_key, stream, lo, hi, ack.as_str()], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })?;
        rows.collect::<rusqlite::Result<Vec<(i64, String)>>>()?
    };
    for (_, client_id) in &marked {
        project_ack(tx, owner_key, client_id, ack)?;
    }
    let mut raised = {
        let mut stmt = tx.prepare(
            "UPDATE messages SET send_state = ?5 WHERE owner_key = ?1 \
             AND conversation_id = ?2 AND conversation_type = 'dm' AND sender_key = ?1 \
             AND stream_seq IS NOT NULL AND stream_seq BETWEEN ?3 AND ?4 \
             AND message_id IS NOT NULL AND send_state != 'read' AND send_state != ?5 \
             AND NOT EXISTS (SELECT 1 FROM outbox o \
             WHERE o.owner_key = ?1 AND o.stream = ?6 AND o.stream_seq = messages.stream_seq \
             AND o.state = 'queued') RETURNING stream_seq, message_id",
        )?;
        let rows = stmt.query_map(
            params![owner_key, peer_key, lo, hi, ack.as_str(), stream],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        rows.collect::<rusqlite::Result<Vec<(i64, String)>>>()?
    };
    raised.extend(marked);
    raised.sort();
    tx.execute(
        "DELETE FROM outbox WHERE owner_key = ?1 AND stream = ?2 \
         AND stream_seq BETWEEN ?3 AND ?4 AND state != 'queued'",
        params![owner_key, stream, lo, hi],
    )?;
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
