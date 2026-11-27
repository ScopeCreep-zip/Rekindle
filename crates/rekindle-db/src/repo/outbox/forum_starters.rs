//! Cross-stream dependencies from forum starters to their thread-creation op.

use rusqlite::{params, Transaction};

use super::{reads::query_rows, Row};

const DEPENDENTS: &str = "owner_key = ?1 AND kind = 'channel_post' AND depends_on = ?2";

/// Clear the dependency in the same transaction that settles the governance
/// op sent. A missing governance row alone never makes a starter eligible.
///
/// # Errors
/// The dependent rows could not be updated.
pub fn release_forum_starters(tx: &Transaction<'_>, owner: &str, op: &str) -> rusqlite::Result<()> {
    tx.execute(
        &format!("UPDATE outbox SET depends_on = NULL WHERE {DEPENDENTS}"),
        params![owner, op],
    )?;
    Ok(())
}

/// Remove the unsent starters in the same transaction that records an op's
/// refusal or failure. Returns their rows for the post-commit status events.
///
/// # Errors
/// The dependent rows could not be read or removed.
pub fn abandon_forum_starters(
    tx: &Transaction<'_>,
    owner: &str,
    op: &str,
) -> rusqlite::Result<Vec<Row>> {
    let rows = query_rows(tx, DEPENDENTS, params![owner, op])?;
    if !rows.is_empty() {
        tx.execute(
            &format!("DELETE FROM outbox WHERE {DEPENDENTS}"),
            params![owner, op],
        )?;
    }
    Ok(rows)
}
