//! Received DM plaintext cache.
//!
//! Messages are Signal-decrypted by the node crate and stored here
//! as encrypted plaintext for fast history loads. Rebuildable from
//! the inbound DhtLog + Signal session if the cache is lost.

use rusqlite::params;

use crate::error::StorageResult;
use crate::messages::DmRecord;
use crate::vault::VaultStore;

impl VaultStore {
    /// Store a received DM. Body is entry-encrypted.
    pub fn store_received_dm(
        &self,
        peer_key: &str,
        sender_name: &str,
        body: &str,
        timestamp: u64,
        sequence: u64,
        message_id: &str,
    ) -> StorageResult<()> {
        let ct = self.encrypt_entry(body.as_bytes())?;
        let conn = self.conn();
        conn.execute(
            "INSERT OR IGNORE INTO dm_received
               (peer_key, sender_name, body, timestamp, sequence, message_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![peer_key, sender_name, ct, i64::try_from(timestamp).unwrap_or(i64::MAX), i64::try_from(sequence).unwrap_or(i64::MAX), message_id],
        )?;
        Ok(())
    }

    /// Query most recent N received DMs for a peer, oldest first.
    pub fn query_received_dm(&self, peer_key: &str, limit: u32) -> StorageResult<Vec<DmRecord>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT sender_name, body, timestamp, message_id FROM dm_received
             WHERE peer_key = ?1 ORDER BY timestamp DESC LIMIT ?2",
        )?;
        let mut rows: Vec<DmRecord> = stmt
            .query_map(params![peer_key, i64::from(limit)], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Vec<u8>>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })?
            .filter_map(Result::ok)
            .filter_map(|(name, ct, ts, mid)| {
                let body = String::from_utf8(self.decrypt_entry(&ct).ok()?).ok()?;
                Some(DmRecord {
                    sender_name: name,
                    body,
                    timestamp: u64::try_from(ts).unwrap_or(0),
                    message_id: mid,
                    is_self: false,
                })
            })
            .collect();
        rows.reverse();
        Ok(rows)
    }

    /// Unified DM thread: merge sent + received, sorted by timestamp, capped at limit.
    pub fn query_dm_thread(&self, peer_key: &str, limit: u32) -> StorageResult<Vec<DmRecord>> {
        let mut merged = self.query_sent_dm(peer_key, limit)?;
        merged.extend(self.query_received_dm(peer_key, limit)?);
        merged.sort_by_key(|r| r.timestamp);
        if merged.len() > limit as usize {
            merged.drain(..merged.len() - limit as usize);
        }
        Ok(merged)
    }
}
