//! Sent DM plaintext storage.
//!
//! Forward secrecy means we cannot re-decrypt our own outbound Signal
//! ciphertext — the ratchet keys are consumed. This store persists sent
//! message plaintext so the user can see their own messages in history.

use rusqlite::params;

use crate::error::StorageResult;
use crate::messages::DmRecord;
use crate::vault::VaultStore;

impl VaultStore {
    /// Store a sent DM. Body is entry-encrypted before storage.
    pub fn store_sent_dm(
        &self,
        peer_key: &str,
        body: &str,
        timestamp: u64,
        message_id: &str,
    ) -> StorageResult<()> {
        let ct = self.encrypt_entry(body.as_bytes())?;
        let conn = self.conn();
        conn.execute(
            "INSERT OR IGNORE INTO dm_sent (peer_key, body, timestamp, message_id)
             VALUES (?1, ?2, ?3, ?4)",
            params![peer_key, ct, i64::try_from(timestamp).unwrap_or(i64::MAX), message_id],
        )?;
        Ok(())
    }

    /// Query most recent N sent DMs for a peer, oldest first.
    pub fn query_sent_dm(&self, peer_key: &str, limit: u32) -> StorageResult<Vec<DmRecord>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT body, timestamp, message_id FROM dm_sent
             WHERE peer_key = ?1 ORDER BY timestamp DESC LIMIT ?2",
        )?;
        let mut rows: Vec<DmRecord> = stmt
            .query_map(params![peer_key, i64::from(limit)], |row| {
                Ok((
                    row.get::<_, Vec<u8>>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })?
            .filter_map(Result::ok)
            .filter_map(|(ct, ts, mid)| {
                let body = String::from_utf8(self.decrypt_entry(&ct).ok()?).ok()?;
                Some(DmRecord {
                    sender_name: "you".into(),
                    body,
                    timestamp: u64::try_from(ts).unwrap_or(0),
                    message_id: mid,
                    is_self: true,
                })
            })
            .collect();
        rows.reverse(); // oldest first
        Ok(rows)
    }
}
