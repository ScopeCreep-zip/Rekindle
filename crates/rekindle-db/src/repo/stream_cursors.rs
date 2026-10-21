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
}
