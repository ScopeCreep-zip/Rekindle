//! Skipped message key storage (HE-DR).
//!
//! In the Header-Encrypted Double Ratchet, skipped message keys are
//! indexed by `(header_key, counter)` because the receiver doesn't know
//! the sender's DH public key until after header decryption.
//!
//! Skipped keys are entry-encrypted at rest. Lookup requires a bounded
//! scan (MAX per session = 2000) with per-entry decrypt + compare.
//! Worst case ~100ms at ~50μs per AES-GCM decrypt. Acceptable for a
//! per-message operation that only fires on out-of-order delivery.

use rusqlite::params;
use zeroize::Zeroizing;

use crate::error::{StorageError, StorageResult};
use crate::sessions::SessionId;
use crate::vault::VaultStore;
use crate::vault::schema::timestamp_secs;

pub const MAX_SKIP_PER_SESSION: u32 = 2000;
pub const SKIPPED_TTL_SECS: i64 = 7 * 86400;

impl VaultStore {
    /// Store a skipped message key. Enforces per-session cap.
    pub fn store_skipped_key(
        &self,
        session_id: &SessionId,
        header_key: &[u8; 32],
        counter: u32,
        message_key: &[u8; 32],
    ) -> StorageResult<()> {
        let conn = self.conn();
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM skipped_keys WHERE session_id = ?1",
            params![session_id.as_slice()],
            |row| row.get(0),
        )?;
        if count >= i64::from(MAX_SKIP_PER_SESSION) {
            return Err(StorageError::SkippedKeyLimit {
                max: MAX_SKIP_PER_SESSION,
            });
        }

        let hk_ct = self.encrypt_entry(header_key)?;
        let mk_ct = self.encrypt_entry(message_key)?;

        conn.execute(
            "INSERT OR IGNORE INTO skipped_keys
               (session_id, header_key, counter, message_key, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                session_id.as_slice(),
                hk_ct,
                i64::from(counter),
                mk_ct,
                timestamp_secs(),
            ],
        )?;
        Ok(())
    }

    /// Look up and consume a skipped key. Returns `None` if not found.
    /// The key is deleted immediately — single use. Expired keys found
    /// during the scan are opportunistically cleaned up.
    pub fn take_skipped_key(
        &self,
        session_id: &SessionId,
        header_key: &[u8; 32],
        counter: u32,
    ) -> StorageResult<Option<Zeroizing<[u8; 32]>>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT rowid, header_key, counter, message_key, created_at
             FROM skipped_keys WHERE session_id = ?1",
        )?;

        let now = timestamp_secs();
        let mut found: Option<(i64, Zeroizing<[u8; 32]>)> = None;
        let mut expired_rowids = Vec::new();

        let rows = stmt.query_map(params![session_id.as_slice()], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Vec<u8>>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, Vec<u8>>(3)?,
                row.get::<_, i64>(4)?,
            ))
        })?;

        for row in rows {
            let (rowid, hk_ct, ct, mk_ct, created_at) = row?;

            if now - created_at > SKIPPED_TTL_SECS {
                expired_rowids.push(rowid);
                continue;
            }

            if found.is_some() {
                continue; // already matched, just collecting expired
            }

            if let Ok(hk_plain) = self.decrypt_entry(&hk_ct) {
                if hk_plain.len() == 32
                    && hk_plain.as_slice() == header_key
                    && ct == i64::from(counter)
                {
                    let mk_plain = self.decrypt_entry(&mk_ct)?;
                    if mk_plain.len() != 32 {
                        return Err(StorageError::VaultCorrupt {
                            reason: "skipped message_key not 32 bytes".into(),
                        });
                    }
                    let mut mk = Zeroizing::new([0u8; 32]);
                    mk.copy_from_slice(&mk_plain);
                    found = Some((rowid, mk));
                }
            }
        }

        // Delete matched key (single-use) and expired keys
        if let Some((found_rowid, mk)) = found {
            conn.execute(
                "DELETE FROM skipped_keys WHERE rowid = ?1",
                params![found_rowid],
            )?;
            for rid in &expired_rowids {
                conn.execute("DELETE FROM skipped_keys WHERE rowid = ?1", params![rid])?;
            }
            return Ok(Some(mk));
        }

        for rid in &expired_rowids {
            conn.execute("DELETE FROM skipped_keys WHERE rowid = ?1", params![rid])?;
        }

        Ok(None)
    }

    /// Delete all skipped keys for a session.
    pub fn clear_skipped_keys(&self, session_id: &SessionId) -> StorageResult<()> {
        let conn = self.conn();
        conn.execute(
            "DELETE FROM skipped_keys WHERE session_id = ?1",
            params![session_id.as_slice()],
        )?;
        Ok(())
    }

    /// Count skipped keys for a specific session.
    pub fn count_skipped_keys_for_session(
        &self,
        session_id: &SessionId,
    ) -> StorageResult<u64> {
        let conn = self.conn();
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM skipped_keys WHERE session_id = ?1",
            params![session_id.as_slice()],
            |row| row.get(0),
        )?;
        Ok(u64::try_from(count).unwrap_or(0))
    }
}
