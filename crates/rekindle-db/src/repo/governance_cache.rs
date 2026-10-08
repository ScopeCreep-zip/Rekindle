//! `governance_entries_cache`: the CRDT merge input per community, kept so
//! a login can re-merge governance before the DHT rebuild finishes.
//!
//! The rows hold the lossless per-author entry sets, not a merged view:
//! merge is reader-validates, so re-merging the input reproduces the same
//! state and a denormalized table would not.

use rekindle_types::governance::GovernanceEntry;
use rekindle_types::id::PseudonymKey;
use rusqlite::{params, Connection};

/// One community's cached merge input.
pub type Entries = Vec<(PseudonymKey, Vec<GovernanceEntry>)>;

/// Every cached community for `owner_key`. A row that no longer decodes is
/// logged and skipped: the DHT rebuild replaces it.
///
/// # Errors
/// The query failed.
pub fn load(conn: &Connection, owner_key: &str) -> rusqlite::Result<Vec<(String, Entries)>> {
    let mut stmt = conn.prepare(
        "SELECT community_id, entries_json FROM governance_entries_cache WHERE owner_key = ?1",
    )?;
    let rows = stmt
        .query_map(params![owner_key], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows
        .into_iter()
        .filter_map(|(community_id, json)| match serde_json::from_str(&json) {
            Ok(entries) => Some((community_id, entries)),
            Err(error) => {
                tracing::warn!(
                    community = %community_id,
                    %error,
                    "skipping corrupt governance cache row",
                );
                None
            }
        })
        .collect())
}

/// Replace `community_id`'s cached merge input.
///
/// # Errors
/// The entries do not serialize, or the write failed.
pub fn save(
    conn: &Connection,
    owner_key: &str,
    community_id: &str,
    entries: &[(PseudonymKey, Vec<GovernanceEntry>)],
    updated_at: i64,
) -> rusqlite::Result<()> {
    let json = serde_json::to_string(entries)
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
    conn.execute(
        "INSERT INTO governance_entries_cache (owner_key, community_id, entries_json, updated_at)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(owner_key, community_id)
         DO UPDATE SET entries_json = excluded.entries_json, updated_at = excluded.updated_at",
        params![owner_key, community_id, json, updated_at],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::fixture::{self, OWNER};

    #[test]
    fn a_saved_entry_set_loads_back_and_a_resave_replaces_it() {
        let conn = fixture::conn();
        let first = vec![(PseudonymKey([1; 32]), Vec::new())];
        save(&conn, OWNER, "c1", &first, 1).unwrap();
        let second = vec![(PseudonymKey([2; 32]), Vec::new())];
        save(&conn, OWNER, "c1", &second, 2).unwrap();
        let loaded = load(&conn, OWNER).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].0, "c1");
        assert_eq!(loaded[0].1[0].0, PseudonymKey([2; 32]));
    }

    #[test]
    fn a_corrupt_row_is_skipped() {
        let conn = fixture::conn();
        save(&conn, OWNER, "good", &[], 1).unwrap();
        conn.execute(
            "INSERT INTO governance_entries_cache VALUES (?1, 'bad', 'not json', 1)",
            [OWNER],
        )
        .unwrap();
        let loaded = load(&conn, OWNER).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].0, "good");
    }
}
