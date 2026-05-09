//! Pending outbound DhtLog key bridge.
//!
//! At friend request send time, we know the target's profile DHT key
//! but not their Ed25519 public key. The outbound DhtLog key is stored
//! here keyed by profile DHT key. When the acceptance is discovered
//! (which carries the peer's Ed25519 key), we consume (`take`) the
//! outbound log key and create the dm_peers entry under the correct
//! Ed25519 key in session metadata.

use rusqlite::{params, OptionalExtension};

use crate::error::StorageResult;
use crate::vault::VaultStore;
use crate::vault::schema::timestamp_secs;

impl VaultStore {
    pub fn store_pending_outbound(
        &self,
        target_profile_key: &str,
        outbound_log_key: &str,
    ) -> StorageResult<()> {
        let conn = self.conn();
        conn.execute(
            "INSERT INTO pending_outbound_logs
               (target_profile_key, outbound_log_key, created_at)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(target_profile_key) DO UPDATE SET outbound_log_key = ?2",
            params![target_profile_key, outbound_log_key, timestamp_secs()],
        )?;
        Ok(())
    }

    /// Consume the pending outbound log key. Deletes on retrieval.
    pub fn take_pending_outbound(
        &self,
        target_profile_key: &str,
    ) -> StorageResult<Option<String>> {
        let conn = self.conn();
        let key: Option<String> = conn
            .query_row(
                "SELECT outbound_log_key FROM pending_outbound_logs
                 WHERE target_profile_key = ?1",
                params![target_profile_key],
                |row| row.get(0),
            )
            .optional()?;
        if key.is_some() {
            conn.execute(
                "DELETE FROM pending_outbound_logs WHERE target_profile_key = ?1",
                params![target_profile_key],
            )?;
        }
        Ok(key)
    }

    /// List all pending outbound entries (for diagnostics/status).
    pub fn list_pending_outbound(&self) -> StorageResult<Vec<(String, String)>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT target_profile_key, outbound_log_key FROM pending_outbound_logs",
        )?;
        let pairs: Vec<(String, String)> = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .filter_map(Result::ok)
            .collect();
        Ok(pairs)
    }
}
