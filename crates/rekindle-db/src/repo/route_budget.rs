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
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )
    .optional()
}

/// Store the identity's bucket after a spend or its first unlock.
///
/// # Errors
/// The write failed.
pub fn save(conn: &Connection, owner_key: &str, row: BudgetRow) -> rusqlite::Result<()> {
    let (tokens, refilled_ms, week_start_ms, week_spent) = row;
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
