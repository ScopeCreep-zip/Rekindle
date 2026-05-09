//! Friend display name mapping.
//!
//! Non-secret metadata populated on friend accept, used for DM sender
//! resolution in the TUI. Not entry-encrypted (display names are metadata,
//! not secrets). Still protected by SQLCipher page encryption.

use std::collections::HashMap;
use rusqlite::params;

use crate::error::StorageResult;
use crate::vault::VaultStore;
use crate::vault::schema::timestamp_secs;

impl VaultStore {
    pub fn store_friend_name(&self, peer_key: &str, display_name: &str) -> StorageResult<()> {
        let conn = self.conn();
        conn.execute(
            "INSERT INTO friend_names (peer_key, display_name, updated_at)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(peer_key) DO UPDATE SET display_name = ?2, updated_at = ?3",
            params![peer_key, display_name, timestamp_secs()],
        )?;
        Ok(())
    }

    pub fn load_friend_names(&self) -> StorageResult<HashMap<String, String>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT peer_key, display_name FROM friend_names")?;
        let map: HashMap<String, String> = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .filter_map(Result::ok)
            .collect();
        Ok(map)
    }

    pub fn resolve_friend_name(&self, peer_key: &str) -> StorageResult<Option<String>> {
        let conn = self.conn();
        let name: Option<String> = conn
            .query_row(
                "SELECT display_name FROM friend_names WHERE peer_key = ?1",
                params![peer_key],
                |row| row.get(0),
            )
            .optional()?;
        Ok(name)
    }

    pub fn delete_friend_name(&self, peer_key: &str) -> StorageResult<()> {
        let conn = self.conn();
        conn.execute(
            "DELETE FROM friend_names WHERE peer_key = ?1",
            params![peer_key],
        )?;
        Ok(())
    }
}

use rusqlite::OptionalExtension;
