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

/// Move the cursor of `writer` on `stream` up to `seq`; a lower or equal
/// `seq` changes nothing. Whether it moved.
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
    let seq = i64::try_from(seq).map_err(|e| rusqlite::Error::ToSqlConversionFailure(e.into()))?;
    let changed = conn.execute(
        "INSERT INTO stream_cursors (owner_key, stream, writer, seq, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5) \
         ON CONFLICT(owner_key, stream, writer) DO UPDATE SET seq = ?4, updated_at = ?5 \
         WHERE stream_cursors.seq < ?4",
        params![owner_key, stream, writer, seq, now],
    )?;
    Ok(changed > 0)
}

/// Forget every cursor on `writer` (their record was replaced, or they are
/// no friend now).
///
/// # Errors
/// The write failed.
pub fn reset(
    conn: &Connection,
    owner_key: &str,
    stream: &str,
    writer: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "DELETE FROM stream_cursors WHERE owner_key = ?1 AND stream = ?2 AND writer = ?3",
        params![owner_key, stream, writer],
    )?;
    Ok(())
}

/// D23's receiver rule on an applied entry's chain `seq` from `writer` on
/// `stream` (live A/B finding 71 refined), whatever path brought it: the
/// cursor's next advances the cursor; one at or below it is a duplicate;
/// one past a gap is applied and the gap held for the chain diff, the
/// cursor not moved past it. Returns the cursor before the step.
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
