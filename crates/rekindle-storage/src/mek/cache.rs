//! MEK vault operations — store, load all, delete by community.

use rusqlite::params;

use crate::error::StorageResult;
use crate::mek::MekEntry;
use crate::vault::VaultStore;
use crate::vault::schema::timestamp_secs;

impl VaultStore {
    /// Persist a MEK. Entry-encrypted before storage.
    pub fn store_mek(
        &self,
        community_id: &str,
        channel_id: &str,
        generation: u64,
        key_bytes: &[u8; 32],
    ) -> StorageResult<()> {
        let ct = self.encrypt_entry(key_bytes)?;
        let conn = self.conn();
        conn.execute(
            "INSERT INTO mek_cache
               (community_id, channel_id, generation, key_bytes, cached_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT DO UPDATE SET key_bytes = ?4, cached_at = ?5",
            params![community_id, channel_id, i64::try_from(generation).unwrap_or(i64::MAX), ct, timestamp_secs()],
        )?;
        Ok(())
    }

    /// Load all MEKs. Called once on daemon unlock to warm the in-memory cache.
    pub fn load_all_meks(&self) -> StorageResult<Vec<MekEntry>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT community_id, channel_id, generation, key_bytes FROM mek_cache",
        )?;
        let entries: Vec<MekEntry> = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, Vec<u8>>(3)?,
                ))
            })?
            .filter_map(Result::ok)
            .filter_map(|(cid, chid, gen, ct)| {
                let plain = self.decrypt_entry(&ct).ok()?;
                let key: [u8; 32] = plain.try_into().ok()?;
                Some(MekEntry {
                    community_id: cid,
                    channel_id: chid,
                    generation: u64::try_from(gen).unwrap_or(0),
                    key_bytes: key,
                })
            })
            .collect();
        Ok(entries)
    }

    /// Delete all MEKs for a community (on leave/eviction).
    pub fn delete_community_meks(&self, community_id: &str) -> StorageResult<()> {
        let conn = self.conn();
        conn.execute(
            "DELETE FROM mek_cache WHERE community_id = ?1",
            params![community_id],
        )?;
        Ok(())
    }

    /// Count of cached MEK entries across all communities.
    pub fn count_meks(&self) -> StorageResult<u64> {
        let conn = self.conn();
        let count: i64 =
            conn.query_row("SELECT COUNT(*) FROM mek_cache", [], |row| row.get(0))?;
        Ok(u64::try_from(count).unwrap_or(0))
    }
}
