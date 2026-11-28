//! `route_budget`: the identity's new-route budget (plan RV1, threat model
//! L2 bounded exposure), the bucket `rekindle_route::budget` spends from,
//! and the friend code slot's roll budget (plan RV2e), a bucket of the same
//! shape.

use rusqlite::{params, Connection, OptionalExtension as _};

/// The persisted bucket: `(tokens, refilled_ms, week_start_ms, week_spent)`.
pub type BudgetRow = (u32, u64, u64, u32);

/// Which of the identity's buckets a row holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BudgetKind {
    /// New routes whose blob leaves the device (plan RV1).
    Routes,
    /// Friend code slot rolls (plan RV2e).
    CodeRolls,
}

impl BudgetKind {
    /// The row's `kind` column.
    const fn as_sql(self) -> &'static str {
        match self {
            Self::Routes => "routes",
            Self::CodeRolls => "code_rolls",
        }
    }
}

/// The identity's bucket `kind`, or `None` before its first unlock.
///
/// # Errors
/// The query failed.
pub fn load(
    conn: &Connection,
    owner_key: &str,
    kind: BudgetKind,
) -> rusqlite::Result<Option<BudgetRow>> {
    conn.query_row(
        "SELECT tokens, refilled_ms, week_start_ms, week_spent FROM route_budget \
         WHERE owner_key = ?1 AND kind = ?2",
        params![owner_key, kind.as_sql()],
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

/// Store the identity's bucket `kind` after a spend or its first unlock.
///
/// # Errors
/// The write failed.
pub fn save(
    conn: &Connection,
    owner_key: &str,
    kind: BudgetKind,
    row: BudgetRow,
) -> rusqlite::Result<()> {
    let (tokens, refilled_ms, week_start_ms, week_spent) = row;
    let (refilled_ms, week_start_ms) = (ms_to_sql(refilled_ms)?, ms_to_sql(week_start_ms)?);
    conn.execute(
        "INSERT INTO route_budget \
         (owner_key, kind, tokens, refilled_ms, week_start_ms, week_spent) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
         ON CONFLICT(owner_key, kind) DO UPDATE SET tokens = excluded.tokens, \
         refilled_ms = excluded.refilled_ms, week_start_ms = excluded.week_start_ms, \
         week_spent = excluded.week_spent",
        params![
            owner_key,
            kind.as_sql(),
            tokens,
            refilled_ms,
            week_start_ms,
            week_spent
        ],
    )?;
    Ok(())
}

#[cfg(test)]
#[path = "route_budget_tests.rs"]
mod tests;
