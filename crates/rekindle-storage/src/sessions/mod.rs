//! Triple Ratchet session storage.
//!
//! Session data is opaque bytes — the node crate serializes
//! `TripleRatchetSession` to CBOR, passes bytes to `store_session`,
//! and gets bytes back from `load_session` which it deserializes.
//! This crate never imports or knows about `TripleRatchetSession`.

pub mod skipped;
pub mod eviction;

use rusqlite::{params, OptionalExtension};

use crate::error::{StorageError, StorageResult};
use crate::vault::VaultStore;
use crate::vault::schema::timestamp_secs;

/// 32-byte session identifier: BLAKE3(IK_A || IK_B || nonce).
pub type SessionId = [u8; 32];

impl VaultStore {
    /// Store or update a session. Session data is entry-encrypted.
    pub fn store_session(
        &self,
        session_id: &SessionId,
        peer_key: &str,
        direction: u8,
        session_data: &[u8],
        spqr_active: bool,
        trust_level: u8,
    ) -> StorageResult<()> {
        let ct = self.encrypt_entry(session_data)?;
        let now = timestamp_secs();
        let conn = self.conn();
        conn.execute(
            "INSERT INTO ratchet_sessions
               (session_id, peer_key, direction, session_data,
                spqr_active, trust_level, last_active, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)
             ON CONFLICT(session_id) DO UPDATE SET
                session_data = ?4, spqr_active = ?5,
                trust_level = ?6, last_active = ?7",
            params![
                session_id.as_slice(),
                peer_key,
                i32::from(direction),
                ct,
                i32::from(u8::from(spqr_active)),
                i32::from(trust_level),
                now,
            ],
        )?;
        Ok(())
    }

    /// Load a session by ID. Returns decrypted CBOR bytes or `None`.
    pub fn load_session(&self, session_id: &SessionId) -> StorageResult<Option<Vec<u8>>> {
        let conn = self.conn();
        let ct: Option<Vec<u8>> = conn
            .query_row(
                "SELECT session_data FROM ratchet_sessions WHERE session_id = ?1",
                params![session_id.as_slice()],
                |row| row.get(0),
            )
            .optional()?;
        match ct {
            Some(ct) => Ok(Some(self.decrypt_entry(&ct)?)),
            None => Ok(None),
        }
    }

    /// Load the most-recently-active session for a peer.
    pub fn load_session_by_peer(
        &self,
        peer_key: &str,
    ) -> StorageResult<Option<(SessionId, Vec<u8>)>> {
        let conn = self.conn();
        let row: Option<(Vec<u8>, Vec<u8>)> = conn
            .query_row(
                "SELECT session_id, session_data FROM ratchet_sessions
                 WHERE peer_key = ?1 ORDER BY last_active DESC LIMIT 1",
                params![peer_key],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        match row {
            Some((id_vec, ct)) => {
                let id: SessionId =
                    id_vec.try_into().map_err(|_| StorageError::VaultCorrupt {
                        reason: "session_id not 32 bytes".into(),
                    })?;
                let data = self.decrypt_entry(&ct)?;
                Ok(Some((id, data)))
            }
            None => Ok(None),
        }
    }

    /// Delete a session and all its skipped keys.
    pub fn delete_session(&self, session_id: &SessionId) -> StorageResult<()> {
        let conn = self.conn();
        conn.execute(
            "DELETE FROM ratchet_sessions WHERE session_id = ?1",
            params![session_id.as_slice()],
        )?;
        conn.execute(
            "DELETE FROM skipped_keys WHERE session_id = ?1",
            params![session_id.as_slice()],
        )?;
        Ok(())
    }

    /// Check whether any session exists for a peer.
    pub fn has_session_for_peer(&self, peer_key: &str) -> StorageResult<bool> {
        let conn = self.conn();
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM ratchet_sessions WHERE peer_key = ?1",
            params![peer_key],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    /// All distinct peer keys with sessions, ordered by most recent activity.
    pub fn list_session_peers(&self) -> StorageResult<Vec<String>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT DISTINCT peer_key FROM ratchet_sessions ORDER BY last_active DESC",
        )?;
        let peers: Vec<String> = stmt
            .query_map([], |row| row.get(0))?
            .filter_map(Result::ok)
            .collect();
        Ok(peers)
    }

    /// Total session count.
    pub fn count_sessions(&self) -> StorageResult<u64> {
        let conn = self.conn();
        let count: i64 =
            conn.query_row("SELECT COUNT(*) FROM ratchet_sessions", [], |row| row.get(0))?;
        Ok(u64::try_from(count).unwrap_or(0))
    }
}
