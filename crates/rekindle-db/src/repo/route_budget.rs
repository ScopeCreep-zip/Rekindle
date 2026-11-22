//! `route_budget`: the identity's new-route budget (plan RV1, threat model
//! L2 bounded exposure), the bucket `rekindle_route::budget` spends from.

use rusqlite::{params, Connection, OptionalExtension as _};

/// The persisted bucket: `(tokens, refilled_ms, week_start_ms, week_spent)`.
pub type BudgetRow = (u32, u64, u64, u32);

/// The identity's bucket, or `None` before its first unlock.
///
/// # Errors
/// The query failed.
pub fn load(conn: &Connection, owner_key: &str) -> rusqlite::Result<Option<BudgetRow>> {
    conn.query_row(
        "SELECT tokens, refilled_ms, week_start_ms, week_spent FROM route_budget \
         WHERE owner_key = ?1",
        params![owner_key],
        |row| {
            Ok((
                row.get(0)?,
                ms_from_sql(row.get(1)?, 1)?,
                ms_from_sql(row.get(2)?, 2)?,
                row.get(3)?,
            ))
        },
    )
    .optional()
}

/// A stored millisecond stamp as `u64`. SQLite holds it as `i64`; a negative
/// value is corrupt and fails the read (the Host then attaches an unreadable
/// budget, paused, never a silently refilled one).
fn ms_from_sql(stored: i64, column: usize) -> rusqlite::Result<u64> {
    u64::try_from(stored).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(column, rusqlite::types::Type::Integer, e.into())
    })
}

/// A millisecond stamp as SQLite's `i64` (the `chain_entries` convention).
fn ms_to_sql(ms: u64) -> rusqlite::Result<i64> {
    i64::try_from(ms).map_err(|e| rusqlite::Error::ToSqlConversionFailure(e.into()))
}

/// Store the identity's bucket after a spend or its first unlock.
///
/// # Errors
/// The write failed.
pub fn save(conn: &Connection, owner_key: &str, row: BudgetRow) -> rusqlite::Result<()> {
    let (tokens, refilled_ms, week_start_ms, week_spent) = row;
    let (refilled_ms, week_start_ms) = (ms_to_sql(refilled_ms)?, ms_to_sql(week_start_ms)?);
    conn.execute(
        "INSERT INTO route_budget (owner_key, tokens, refilled_ms, week_start_ms, week_spent) \
         VALUES (?1, ?2, ?3, ?4, ?5) \
         ON CONFLICT(owner_key) DO UPDATE SET tokens = excluded.tokens, \
         refilled_ms = excluded.refilled_ms, week_start_ms = excluded.week_start_ms, \
         week_spent = excluded.week_spent",
        params![owner_key, tokens, refilled_ms, week_start_ms, week_spent],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::fixture::{self, OWNER};

    /// A bucket saved after a spend reads back unchanged, and before the
    /// first unlock there is none.
    #[test]
    fn a_saved_bucket_reads_back_unchanged() {
        let conn = fixture::conn();
        assert_eq!(load(&conn, OWNER).unwrap(), None);
        let row = (11, 1_763_850_000_000, 1_763_500_000_000, 13);
        save(&conn, OWNER, row).unwrap();
        assert_eq!(load(&conn, OWNER).unwrap(), Some(row));
    }

    /// A stamp SQLite can't hold as `i64` fails the write instead of
    /// wrapping into a negative (and later unreadable) value.
    #[test]
    fn a_stamp_past_i64_fails_the_write() {
        let conn = fixture::conn();
        assert!(save(&conn, OWNER, (1, u64::MAX, 0, 0)).is_err());
        assert_eq!(load(&conn, OWNER).unwrap(), None);
    }
}
