//! Channel message cache.
//!
//! MEK-decrypted by the node crate, stored here entry-encrypted for
//! fast history loads. Rebuildable from the DhtLog + MEK if lost.

use rusqlite::params;

use crate::error::StorageResult;
use crate::messages::ChannelRecord;
use crate::vault::VaultStore;

impl VaultStore {
    /// Store a decrypted channel message.
    #[allow(clippy::too_many_arguments)]
    pub fn store_channel_message(
        &self,
        community_id: &str,
        channel_id: &str,
        author_pseudonym: &str,
        author_display_name: &str,
        body: &str,
        timestamp: u64,
        sequence: u64,
        message_id: &str,
        mek_generation: u64,
    ) -> StorageResult<()> {
        let ct = self.encrypt_entry(body.as_bytes())?;
        let conn = self.conn();
        conn.execute(
            "INSERT OR IGNORE INTO channel_messages
               (community_id, channel_id, author_pseudonym, author_display_name,
                body, timestamp, sequence, message_id, mek_generation)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                community_id,
                channel_id,
                author_pseudonym,
                author_display_name,
                ct,
                i64::try_from(timestamp).unwrap_or(i64::MAX),
                i64::try_from(sequence).unwrap_or(i64::MAX),
                message_id,
                i64::try_from(mek_generation).unwrap_or(i64::MAX),
            ],
        )?;
        Ok(())
    }

    /// Query most recent N channel messages, oldest first.
    pub fn query_channel_history(
        &self,
        community_id: &str,
        channel_id: &str,
        limit: u32,
    ) -> StorageResult<Vec<ChannelRecord>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT author_pseudonym, author_display_name, body, timestamp,
                    sequence, message_id, mek_generation
             FROM channel_messages
             WHERE community_id = ?1 AND channel_id = ?2
             ORDER BY timestamp DESC LIMIT ?3",
        )?;
        let mut rows: Vec<ChannelRecord> = stmt
            .query_map(
                params![community_id, channel_id, i64::from(limit)],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Vec<u8>>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, i64>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, i64>(6)?,
                    ))
                },
            )?
            .filter_map(Result::ok)
            .filter_map(|(ap, adn, ct, ts, seq, mid, mg)| {
                let body = String::from_utf8(self.decrypt_entry(&ct).ok()?).ok()?;
                Some(ChannelRecord {
                    author_pseudonym: ap,
                    author_display_name: adn,
                    body,
                    timestamp: u64::try_from(ts).unwrap_or(0),
                    sequence: u64::try_from(seq).unwrap_or(0),
                    message_id: mid,
                    mek_generation: u64::try_from(mg).unwrap_or(0),
                })
            })
            .collect();
        rows.reverse();
        Ok(rows)
    }

    /// Delete all cached messages for a community (on leave/eviction).
    pub fn delete_community_messages(&self, community_id: &str) -> StorageResult<()> {
        let conn = self.conn();
        conn.execute(
            "DELETE FROM channel_messages WHERE community_id = ?1",
            params![community_id],
        )?;
        Ok(())
    }
}
