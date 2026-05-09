//! Session and skipped-key eviction.
//!
//! - Expired skipped keys: swept by TTL index, O(expired).
//! - Inactive sessions: evicted oldest-first when count exceeds cap.
//! - The node crate runs these from a maintenance task (hourly for TTL,
//!   on-demand for session cap).

use rusqlite::params;

use crate::error::StorageResult;
use crate::sessions::skipped::SKIPPED_TTL_SECS;
use crate::vault::VaultStore;
use crate::vault::schema::timestamp_secs;

impl VaultStore {
    /// Sweep expired skipped keys across ALL sessions.
    /// Returns the number of keys deleted.
    pub fn sweep_expired_skipped_keys(&self) -> StorageResult<u64> {
        let cutoff = timestamp_secs() - SKIPPED_TTL_SECS;
        let conn = self.conn();
        let deleted = conn.execute(
            "DELETE FROM skipped_keys WHERE created_at < ?1",
            params![cutoff],
        )?;
        if deleted > 0 {
            tracing::info!(deleted, "swept expired skipped keys");
        }
        Ok(u64::try_from(deleted).unwrap_or(0))
    }

    /// Evict the oldest inactive sessions until count <= `max_sessions`.
    /// Returns session IDs of evicted sessions so the caller can drop
    /// them from the in-memory DashMap + LRU cache.
    pub fn evict_oldest_sessions(&self, max_sessions: u64) -> StorageResult<Vec<[u8; 32]>> {
        let count = self.count_sessions()?;
        if count <= max_sessions {
            return Ok(Vec::new());
        }

        let to_evict = count - max_sessions;
        let conn = self.conn();

        let mut stmt =
            conn.prepare("SELECT session_id FROM ratchet_sessions ORDER BY last_active ASC LIMIT ?1")?;

        let ids: Vec<Vec<u8>> = stmt
            .query_map(params![i64::try_from(to_evict).unwrap_or(i64::MAX)], |row| row.get(0))?
            .filter_map(Result::ok)
            .collect();

        let mut evicted = Vec::with_capacity(ids.len());
        for id_vec in ids {
            if let Ok(id) = <[u8; 32]>::try_from(id_vec.as_slice()) {
                conn.execute(
                    "DELETE FROM ratchet_sessions WHERE session_id = ?1",
                    params![id.as_slice()],
                )?;
                conn.execute(
                    "DELETE FROM skipped_keys WHERE session_id = ?1",
                    params![id.as_slice()],
                )?;
                evicted.push(id);
            }
        }

        if !evicted.is_empty() {
            tracing::info!(count = evicted.len(), "evicted inactive sessions");
        }
        Ok(evicted)
    }

    /// Total count of skipped keys across all sessions.
    pub fn count_all_skipped_keys(&self) -> StorageResult<u64> {
        let conn = self.conn();
        let count: i64 =
            conn.query_row("SELECT COUNT(*) FROM skipped_keys", [], |row| row.get(0))?;
        Ok(u64::try_from(count).unwrap_or(0))
    }
}
