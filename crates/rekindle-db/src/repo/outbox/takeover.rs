//! A row that takes over another's dependents (plan HOL1, lead decision
//! 47): a first contact still waiting for the peer's prekey record settles
//! `sent` once it commits its PreKeyInit, so the rows that waited for it
//! wait for that PreKeyInit instead (`depends_on`), whose receipt (or the
//! peer's first frame under the session) proves the friendship arrived.

use rusqlite::{params, Connection};

/// Point every row of `stream` whose `depends_on` starts with one of
/// `prefixes` at `client_id` instead (never `client_id` itself); how many
/// moved.
///
/// # Errors
/// The write failed.
pub fn take_over_dependents(
    conn: &Connection,
    owner_key: &str,
    stream: &str,
    prefixes: &[String],
    client_id: &str,
) -> rusqlite::Result<usize> {
    let mut moved = 0;
    for prefix in prefixes {
        moved += conn.execute(
            "UPDATE outbox SET depends_on = ?4 \
             WHERE owner_key = ?1 AND stream = ?2 AND client_id <> ?4 \
               AND depends_on IS NOT NULL \
               AND substr(depends_on, 1, length(?3)) = ?3",
            params![owner_key, stream, prefix, client_id],
        )?;
    }
    Ok(moved)
}
