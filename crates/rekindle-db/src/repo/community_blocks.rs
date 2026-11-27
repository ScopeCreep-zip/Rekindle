//! `community_blocks`: the community members we block, by the pseudonym
//! they post under there (plan E3.1 F3, D26). The channel ingest gate drops
//! a blocked pseudonym's posts; an identity-key block (`blocked`) cannot
//! match one without linking pseudonyms (threat model I8, L1, L3).

use rusqlite::{params, Connection};

/// A blocked pseudonym, newest first in [`list`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommunityBlock {
    pub pseudonym: String,
    pub blocked_at: i64,
}

/// Block `pseudonym` in `community_id` (a repeat updates the time). The
/// pseudonym is stored lowercased.
///
/// # Errors
/// The write failed.
pub fn insert(
    conn: &Connection,
    owner_key: &str,
    community_id: &str,
    pseudonym: &str,
    blocked_at: i64,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO community_blocks (owner_key, community_id, pseudonym, blocked_at) \
         VALUES (?1, ?2, ?3, ?4) \
         ON CONFLICT(owner_key, community_id, pseudonym) DO UPDATE SET \
           blocked_at = excluded.blocked_at",
        params![
            owner_key,
            community_id,
            pseudonym.to_ascii_lowercase(),
            blocked_at
        ],
    )?;
    Ok(())
}

/// Unblock `pseudonym` in `community_id`; whether it was blocked.
///
/// # Errors
/// The write failed.
pub fn delete(
    conn: &Connection,
    owner_key: &str,
    community_id: &str,
    pseudonym: &str,
) -> rusqlite::Result<bool> {
    Ok(conn.execute(
        "DELETE FROM community_blocks \
         WHERE owner_key = ?1 AND community_id = ?2 AND pseudonym = ?3",
        params![owner_key, community_id, pseudonym.to_ascii_lowercase()],
    )? > 0)
}

/// Every pseudonym blocked in `community_id`, newest first.
///
/// # Errors
/// The query failed.
pub fn list(
    conn: &Connection,
    owner_key: &str,
    community_id: &str,
) -> rusqlite::Result<Vec<CommunityBlock>> {
    let mut stmt = conn.prepare(
        "SELECT pseudonym, blocked_at FROM community_blocks \
         WHERE owner_key = ?1 AND community_id = ?2 \
         ORDER BY blocked_at DESC, pseudonym ASC",
    )?;
    let rows = stmt
        .query_map(params![owner_key, community_id], |row| {
            Ok(CommunityBlock {
                pseudonym: row.get(0)?,
                blocked_at: row.get(1)?,
            })
        })?
        .collect();
    rows
}

#[cfg(test)]
#[path = "community_blocks_tests.rs"]
mod tests;
