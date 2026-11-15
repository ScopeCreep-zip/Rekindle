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
mod tests {
    use super::*;
    use crate::repo::fixture::{self, OWNER};

    #[test]
    fn a_cursor_only_moves_up() {
        let conn = fixture::conn();
        assert_eq!(get(&conn, OWNER, "dm", "w").unwrap(), 0);
        assert!(advance(&conn, OWNER, "dm", "w", 3, 1).unwrap());
        assert!(!advance(&conn, OWNER, "dm", "w", 2, 2).unwrap());
        assert!(!advance(&conn, OWNER, "dm", "w", 3, 2).unwrap());
        assert_eq!(get(&conn, OWNER, "dm", "w").unwrap(), 3);
        assert_eq!(get(&conn, OWNER, "dm", "x").unwrap(), 0);
        reset(&conn, OWNER, "dm", "w").unwrap();
        assert_eq!(get(&conn, OWNER, "dm", "w").unwrap(), 0);
    }

    #[test]
    fn a_friendship_that_ends_forgets_its_dm_cursor() {
        let conn = fixture::conn();
        advance(&conn, OWNER, "dm", "w", 3, 1).unwrap();
        advance(&conn, OWNER, "other", "w", 2, 1).unwrap();
        crate::repo::friends::forget(&conn, OWNER, "w").unwrap();
        assert_eq!(get(&conn, OWNER, "dm", "w").unwrap(), 0);
        assert_eq!(get(&conn, OWNER, "other", "w").unwrap(), 2);
    }

    fn friend(conn: &Connection) {
        conn.execute(
            "INSERT INTO friends (owner_key, public_key, added_at, friendship_state) \
             VALUES (?1, 'bob', 0, 'accepted')",
            params![OWNER],
        )
        .unwrap();
    }

    /// Hints 1, 2 and 3 with the friendship's first `StreamRecord` at
    /// either place (it can overtake hint 1 or come after hint 2, live A/B
    /// findings 73 and 77): the cursor ends at 3 and no hint is held.
    #[test]
    fn a_first_stream_record_never_resets_the_hints_cursor() {
        for record_after in [1, 2] {
            let conn = fixture::conn();
            friend(&conn);
            for seq in 1..=3 {
                let at = step(&conn, OWNER, "dm", "bob", seq, 1).unwrap();
                assert_eq!(at, seq - 1, "hint {seq} is the cursor's next, none held");
                if seq == record_after {
                    assert!(adopt_record(&conn, OWNER, "dm", "bob", "rec1").unwrap());
                }
            }
            assert_eq!(get(&conn, OWNER, "dm", "bob").unwrap(), 3);
        }
    }

    fn remembered(conn: &Connection, writer: &str) -> Vec<i64> {
        let mut stmt = conn
            .prepare("SELECT seq FROM stream_applied WHERE writer = ?1 ORDER BY seq")
            .unwrap();
        let rows = stmt.query_map([writer], |r| r.get(0)).unwrap();
        rows.collect::<rusqlite::Result<_>>().unwrap()
    }

    /// Hints 3, 1, 2 (3 overtook both): 3 is applied past the gap and
    /// remembered, 1 steps the cursor, 2 closes the gap and the cursor
    /// absorbs 3 (E2.5i R1-3: it stopped at 2, below a seq applied).
    #[test]
    fn a_closed_gap_absorbs_the_seqs_applied_past_it() {
        let conn = fixture::conn();
        assert_eq!(step(&conn, OWNER, "dm", "w", 3, 1).unwrap(), 0);
        assert_eq!(get(&conn, OWNER, "dm", "w").unwrap(), 0, "past a gap");
        assert_eq!(step(&conn, OWNER, "dm", "w", 1, 2).unwrap(), 0);
        assert_eq!(get(&conn, OWNER, "dm", "w").unwrap(), 1);
        assert_eq!(step(&conn, OWNER, "dm", "w", 2, 3).unwrap(), 1);
        assert_eq!(get(&conn, OWNER, "dm", "w").unwrap(), 3);
        assert!(remembered(&conn, "w").is_empty(), "passed seqs go");
        assert_eq!(get(&conn, OWNER, "dm", "x").unwrap(), 0, "per writer");
    }

    /// A duplicate at or below the cursor changes nothing and is never
    /// remembered.
    #[test]
    fn a_duplicate_below_the_cursor_is_a_no_op() {
        let conn = fixture::conn();
        for seq in 1..=3 {
            step(&conn, OWNER, "dm", "w", seq, 1).unwrap();
        }
        assert_eq!(step(&conn, OWNER, "dm", "w", 2, 2).unwrap(), 3);
        assert_eq!(step(&conn, OWNER, "dm", "w", 3, 2).unwrap(), 3);
        assert_eq!(get(&conn, OWNER, "dm", "w").unwrap(), 3);
        assert!(remembered(&conn, "w").is_empty());
    }

    /// The remembered seqs are rows: a restart (a new connection on the
    /// same database) still absorbs them, and the chain diff's `advance`
    /// absorbs them as the hint's `step` does; a second gap stops it.
    #[test]
    fn a_restart_keeps_the_remembered_seqs() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cursors.db");
        {
            let conn = fixture::conn_at(&path);
            for seq in [2, 3, 5] {
                step(&conn, OWNER, "dm", "w", seq, 1).unwrap();
            }
        }
        let conn = fixture::conn_at(&path);
        assert_eq!(get(&conn, OWNER, "dm", "w").unwrap(), 0);
        assert!(advance(&conn, OWNER, "dm", "w", 1, 2).unwrap());
        assert_eq!(get(&conn, OWNER, "dm", "w").unwrap(), 3, "4 is still a gap");
        assert_eq!(remembered(&conn, "w"), [5]);
        reset(&conn, OWNER, "dm", "w").unwrap();
        assert!(
            remembered(&conn, "w").is_empty(),
            "a new chain forgets them"
        );
    }

    /// A record that replaces the held one is a new chain: the cursor
    /// restarts; the same key again changes nothing.
    #[test]
    fn a_replaced_stream_record_restarts_the_cursor() {
        let conn = fixture::conn();
        friend(&conn);
        assert!(adopt_record(&conn, OWNER, "dm", "bob", "rec1").unwrap());
        step(&conn, OWNER, "dm", "bob", 1, 1).unwrap();
        assert!(!adopt_record(&conn, OWNER, "dm", "bob", "rec1").unwrap());
        assert_eq!(get(&conn, OWNER, "dm", "bob").unwrap(), 1);
        assert!(adopt_record(&conn, OWNER, "dm", "bob", "rec2").unwrap());
        assert_eq!(get(&conn, OWNER, "dm", "bob").unwrap(), 0);
        assert_eq!(
            step(&conn, OWNER, "dm", "bob", 2, 1).unwrap(),
            0,
            "past a gap"
        );
        assert_eq!(get(&conn, OWNER, "dm", "bob").unwrap(), 0);
    }
}
