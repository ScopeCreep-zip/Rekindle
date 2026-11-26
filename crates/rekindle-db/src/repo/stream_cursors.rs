//! `stream_cursors` (plan E2.5, D23): per (stream, writer), the highest
//! sequence number of the writer's chain this identity has passed. The
//! cursor only moves up, and only after the entry's effect commits.

use rusqlite::{params, Connection, OptionalExtension as _};

/// The cursor of `writer` on `stream` (0: nothing passed yet).
///
/// # Errors
/// The query failed.
pub fn get(
    conn: &Connection,
    owner_key: &str,
    stream: &str,
    writer: &str,
) -> rusqlite::Result<u64> {
    let seq: Option<i64> = conn
        .query_row(
            "SELECT seq FROM stream_cursors WHERE owner_key = ?1 AND stream = ?2 AND writer = ?3",
            params![owner_key, stream, writer],
            |row| row.get(0),
        )
        .optional()?;
    Ok(seq.and_then(|s| u64::try_from(s).ok()).unwrap_or(0))
}

/// Move the cursor of `writer` on `stream` up to `seq`, then on through
/// every seq remembered as applied past a gap ([`step`]) that is now
/// contiguous with it; a lower or equal `seq` changes nothing unless a
/// remembered run already joins the cursor. Whether it moved.
///
/// Each write stands alone (the caller may hold a transaction): a crash
/// between them leaves a remembered seq at or below the cursor, which no
/// read counts, and the next move deletes it.
///
/// # Errors
/// `seq` does not fit a row, or the write failed.
pub fn advance(
    conn: &Connection,
    owner_key: &str,
    stream: &str,
    writer: &str,
    seq: u64,
    now: i64,
) -> rusqlite::Result<bool> {
    let at = get(conn, owner_key, stream, writer)?;
    let top = contiguous_top(conn, (owner_key, stream, writer), at.max(seq))?;
    if top <= at {
        return Ok(false);
    }
    let top = i64::try_from(top).map_err(|e| rusqlite::Error::ToSqlConversionFailure(e.into()))?;
    let changed = conn.execute(
        "INSERT INTO stream_cursors (owner_key, stream, writer, seq, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5) \
         ON CONFLICT(owner_key, stream, writer) DO UPDATE SET seq = ?4, updated_at = ?5 \
         WHERE stream_cursors.seq < ?4",
        params![owner_key, stream, writer, top, now],
    )?;
    conn.execute(
        "DELETE FROM stream_applied WHERE owner_key = ?1 AND stream = ?2 AND writer = ?3 \
         AND seq <= ?4",
        params![owner_key, stream, writer, top],
    )?;
    Ok(changed > 0)
}

/// The writer's outbox stream seq of the entry the cursor of `writer` on
/// `stream` was committed on (plan E2.6 G4; 0: none known).
///
/// # Errors
/// The query failed.
pub fn stream_seq(
    conn: &Connection,
    owner_key: &str,
    stream: &str,
    writer: &str,
) -> rusqlite::Result<u64> {
    let seq: Option<i64> = conn
        .query_row(
            "SELECT stream_seq FROM stream_cursors \
             WHERE owner_key = ?1 AND stream = ?2 AND writer = ?3",
            params![owner_key, stream, writer],
            |row| row.get(0),
        )
        .optional()?;
    Ok(seq.and_then(|s| u64::try_from(s).ok()).unwrap_or(0))
}

/// [`advance`] past the applied chain entry `seq`, whose outbox stream seq
/// is `stream_seq` (plan E2.6 G4), and record that stream seq when the
/// cursor stands on the entry. A cursor that absorbed remembered seqs
/// past it keeps the stream seq it held: lower than its entry's, so a
/// later retired stretch needs more held, never less. Whether it moved.
///
/// # Errors
/// A seq does not fit a row, or the write failed.
pub fn advance_entry(
    conn: &Connection,
    (owner_key, stream, writer): (&str, &str, &str),
    (seq, stream_seq): (u64, u64),
    now: i64,
) -> rusqlite::Result<bool> {
    let moved = advance(conn, owner_key, stream, writer, seq, now)?;
    let to_row =
        |v: u64| i64::try_from(v).map_err(|e| rusqlite::Error::ToSqlConversionFailure(e.into()));
    conn.execute(
        "UPDATE stream_cursors SET stream_seq = ?5 \
         WHERE owner_key = ?1 AND stream = ?2 AND writer = ?3 AND seq = ?4",
        params![owner_key, stream, writer, to_row(seq)?, to_row(stream_seq)?],
    )?;
    Ok(moved)
}

/// The highest seq reached from `from` through the remembered applied
/// seqs contiguous with it (`from` itself when none follows it).
fn contiguous_top(
    conn: &Connection,
    (owner_key, stream, writer): (&str, &str, &str),
    from: u64,
) -> rusqlite::Result<u64> {
    let from_i =
        i64::try_from(from).map_err(|e| rusqlite::Error::ToSqlConversionFailure(e.into()))?;
    let mut stmt = conn.prepare_cached(
        "SELECT seq FROM stream_applied WHERE owner_key = ?1 AND stream = ?2 AND writer = ?3 \
         AND seq > ?4 ORDER BY seq",
    )?;
    let seqs = stmt.query_map(params![owner_key, stream, writer, from_i], |row| {
        row.get::<_, i64>(0)
    })?;
    let mut top = from;
    for seq in seqs {
        match u64::try_from(seq?) {
            Ok(seq) if seq == top + 1 => top = seq,
            _ => break,
        }
    }
    Ok(top)
}

/// Forget every cursor on `writer` (their record was replaced, or they are
/// no friend now), with the seqs remembered past its gaps.
///
/// # Errors
/// The write failed.
pub fn reset(
    conn: &Connection,
    owner_key: &str,
    stream: &str,
    writer: &str,
) -> rusqlite::Result<()> {
    for sql in [
        "DELETE FROM stream_cursors WHERE owner_key = ?1 AND stream = ?2 AND writer = ?3",
        "DELETE FROM stream_applied WHERE owner_key = ?1 AND stream = ?2 AND writer = ?3",
    ] {
        conn.execute(sql, params![owner_key, stream, writer])?;
    }
    Ok(())
}

/// Forget every writer's cursor on `stream`, with the seqs remembered past
/// their gaps: an older community key arrived (plan D16, a joiner's
/// history), so every entry passed for want of it is read again. An entry
/// already applied comes back as a duplicate its ingest drops.
///
/// # Errors
/// The write failed.
pub fn reset_stream(conn: &Connection, owner_key: &str, stream: &str) -> rusqlite::Result<()> {
    for sql in [
        "DELETE FROM stream_cursors WHERE owner_key = ?1 AND stream = ?2",
        "DELETE FROM stream_applied WHERE owner_key = ?1 AND stream = ?2",
    ] {
        conn.execute(sql, params![owner_key, stream])?;
    }
    Ok(())
}

/// D23's receiver rule on an applied entry's chain `seq` from `writer` on
/// `stream` (live A/B finding 71 refined; E2.5i R1-3), whatever path
/// brought it: one at or below the cursor is a duplicate and changes
/// nothing; the cursor's next advances the cursor, on through every seq
/// already applied past the gap it closes; one past a gap is applied
/// (the gap held for the chain diff) and remembered, so the cursor is
/// always the highest contiguous seq applied by any path. Returns the
/// cursor before the step.
///
/// # Errors
/// `seq` does not fit a row, or the storage failed.
pub fn step(
    conn: &Connection,
    owner_key: &str,
    stream: &str,
    writer: &str,
    seq: u64,
    now: i64,
) -> rusqlite::Result<u64> {
    let at = get(conn, owner_key, stream, writer)?;
    if seq == at + 1 {
        advance(conn, owner_key, stream, writer, seq, now)?;
    } else if seq > at {
        let seq_i =
            i64::try_from(seq).map_err(|e| rusqlite::Error::ToSqlConversionFailure(e.into()))?;
        conn.execute(
            "INSERT OR IGNORE INTO stream_applied (owner_key, stream, writer, seq) \
             VALUES (?1, ?2, ?3, ?4)",
            params![owner_key, stream, writer, seq_i],
        )?;
    }
    Ok(at)
}

/// Store `peer`'s participant record `key` (their `StreamRecord`) and, on
/// `stream`, keep the cursor on them unless the key replaces one held:
/// the first record names the chain the friendship's hints already
/// stepped the cursor on (live A/B findings 73, 77: resetting it there,
/// after hints 1 and 2, held hint 3 as past a gap), while a replaced
/// record is a new chain, read from its start. An ended friendship's
/// cursor went with its row. Whether the key changed.
///
/// # Errors
/// The storage failed.
pub fn adopt_record(
    conn: &Connection,
    owner_key: &str,
    stream: &str,
    peer: &str,
    key: &str,
) -> rusqlite::Result<bool> {
    let tx = conn.unchecked_transaction()?;
    let replaced =
        !super::friends::peer_participant_record_key(&tx, owner_key, Some(peer))?.is_empty();
    let changed = super::friends::set_peer_participant_record_key(&tx, owner_key, peer, key)?;
    if changed && replaced {
        reset(&tx, owner_key, stream, peer)?;
    }
    tx.commit()?;
    Ok(changed)
}

#[cfg(test)]
#[path = "stream_cursors_tests.rs"]
mod tests;
