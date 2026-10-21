//! `presence_keys`: our friend-presence key (plan E2.5), the newest epoch
//! only.

use rusqlite::{params, Connection, OptionalExtension as _};

/// Our current key: `(epoch, key)`, or `None` before the first is minted.
///
/// # Errors
/// The query failed.
pub fn current(conn: &Connection, owner_key: &str) -> rusqlite::Result<Option<(u32, Vec<u8>)>> {
    conn.query_row(
        "SELECT epoch, key FROM presence_keys WHERE owner_key = ?1 \
         ORDER BY epoch DESC LIMIT 1",
        params![owner_key],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .optional()
}

/// Store the key of `epoch` as our current one; the older epochs go.
///
/// # Errors
/// A write failed.
pub fn replace(
    conn: &Connection,
    owner_key: &str,
    epoch: u32,
    key: &[u8],
    created_at: i64,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO presence_keys (owner_key, epoch, key, created_at) VALUES (?1, ?2, ?3, ?4)",
        params![owner_key, epoch, key, created_at],
    )?;
    conn.execute(
        "DELETE FROM presence_keys WHERE owner_key = ?1 AND epoch < ?2",
        params![owner_key, epoch],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{current, replace};

    #[test]
    fn the_newest_epoch_is_the_only_one_kept() {
        let conn = crate::test_support::schema_conn("aa");
        assert_eq!(current(&conn, "aa").unwrap(), None);
        replace(&conn, "aa", 1, &[1; 32], 10).unwrap();
        replace(&conn, "aa", 2, &[2; 32], 20).unwrap();
        assert_eq!(current(&conn, "aa").unwrap(), Some((2, vec![2; 32])));
        let rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM presence_keys", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 1);
        assert!(
            replace(&conn, "aa", 3, &[3; 16], 30).is_err(),
            "a key is 32 bytes"
        );
    }
}
