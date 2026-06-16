//! DmStore implementation on VaultStore.
//!
//! Each method is a rusqlite query against `dm_conversations` and
//! `dm_messages`. Message body is entry-encrypted before storage
//! (same double-encryption pattern as `dm_sent` / `dm_received`).

use rusqlite::params;

use rekindle_types::dm_store::{
    DmConversation, DmInviteMeta, DmInvitePending, DmMessageInsert,
    DmMessageRecord, DmParticipant, DmSessionMeta, DmStore, DmStoreError,
};

use crate::vault::VaultStore;

impl DmStore for VaultStore {
    fn dm_persist_invite(&self, invite: DmInvitePending) -> Result<(), DmStoreError> {
        let participants_json = serde_json::to_string(&invite.participants)
            .map_err(|e| DmStoreError::InvalidData(format!("participants json: {e}")))?;
        let conn = self.conn();
        conn.execute(
            "INSERT INTO dm_conversations
                (record_key, is_group, initiator_pub_key, initiator_pseudo,
                 my_subkey, participants_json, slot_seed_hex, wrapped_mek_blob,
                 mek_generation, created_at, last_message_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, NULL)
             ON CONFLICT(record_key) DO NOTHING",
            params![
                invite.record_key,
                i64::from(invite.is_group),
                invite.initiator_public_key,
                invite.initiator_pseudonym,
                i64::from(invite.my_subkey),
                participants_json,
                invite.slot_seed_hex,
                invite.wrapped_mek_blob,
                i64::from(invite.mek_generation),
                crate::vault::schema::timestamp_secs(),
            ],
        ).map_err(|e| DmStoreError::Storage(e.to_string()))?;
        Ok(())
    }

    fn dm_list_conversations(&self) -> Result<Vec<DmConversation>, DmStoreError> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT record_key, is_group, initiator_pub_key, initiator_pseudo,
                    my_subkey, participants_json, mek_generation, last_message_at
             FROM dm_conversations
             ORDER BY COALESCE(last_message_at, created_at) DESC",
        ).map_err(|e| DmStoreError::Storage(e.to_string()))?;
        let rows = stmt.query_map([], |row| {
            let pj: String = row.get(5)?;
            let participants: Vec<DmParticipant> =
                serde_json::from_str(&pj).unwrap_or_default();
            Ok(DmConversation {
                record_key: row.get(0)?,
                is_group: { let n: i64 = row.get(1)?; n != 0 },
                initiator_public_key: row.get(2)?,
                initiator_pseudonym: row.get(3)?,
                my_subkey: { let n: i64 = row.get(4)?; u32::try_from(n).unwrap_or(0) },
                participants,
                mek_generation: { let n: i64 = row.get(6)?; u32::try_from(n).unwrap_or(0) },
                last_message_at: {
                    let ts: Option<i64> = row.get(7)?;
                    ts.map(|t| match u64::try_from(t) {
                        Ok(v) => v,
                        Err(_) => {
                            tracing::warn!(raw_ts = t, "dm_list_conversations: negative last_message_at — clock skew");
                            0
                        }
                    })
                },
            })
        }).map_err(|e| DmStoreError::Storage(e.to_string()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| DmStoreError::Storage(e.to_string()))?;
        Ok(rows)
    }

    fn dm_load_messages(
        &self, record_key: &str, limit: u32,
    ) -> Result<Vec<DmMessageRecord>, DmStoreError> {
        tracing::debug!(
            record_key = &record_key[..16.min(record_key.len())],
            limit,
            "dm_load_messages: querying vault"
        );
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT sender_pseudonym, body, timestamp_secs, sequence,
                    mek_generation, is_self
             FROM dm_messages WHERE record_key = ?1
             ORDER BY timestamp_secs DESC LIMIT ?2",
        ).map_err(|e| DmStoreError::Storage(e.to_string()))?;
        let mut rows: Vec<DmMessageRecord> = stmt.query_map(
            params![record_key, i64::from(limit)], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Vec<u8>>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, i64>(5)?,
                ))
            },
        ).map_err(|e| DmStoreError::Storage(e.to_string()))?
        .filter_map(|r| r.ok())
        .filter_map(|(sender, ct, ts, seq, gen, is_self)| {
            let body = String::from_utf8(
                self.decrypt_entry(&ct).ok()?
            ).ok()?;
            Some(DmMessageRecord {
                sender_pseudonym: sender,
                body,
                timestamp: match u64::try_from(ts) {
                    Ok(v) => v,
                    Err(_) => {
                        tracing::warn!(raw_ts = ts, "dm_load_messages: negative timestamp — clock skew or data corruption");
                        0
                    }
                },
                sequence: match u64::try_from(seq) {
                    Ok(v) => v,
                    Err(_) => {
                        tracing::warn!(raw_seq = seq, "dm_load_messages: negative sequence — storage corruption");
                        0
                    }
                },
                mek_generation: match u64::try_from(gen) {
                    Ok(v) => v,
                    Err(_) => {
                        tracing::warn!(raw_gen = gen, "dm_load_messages: negative mek_generation — storage corruption");
                        0
                    }
                },
                is_self: is_self != 0,
            })
        })
        .collect();
        rows.reverse(); // oldest first for UI scrollback
        tracing::info!(
            record_key = &record_key[..16.min(record_key.len())],
            result_count = rows.len(),
            limit,
            "dm_load_messages: query returned"
        );
        Ok(rows)
    }

    fn dm_decline_invite(&self, record_key: &str) -> Result<(), DmStoreError> {
        let conn = self.conn();
        conn.execute(
            "DELETE FROM dm_messages WHERE record_key = ?1",
            params![record_key],
        ).map_err(|e| DmStoreError::Storage(e.to_string()))?;
        conn.execute(
            "DELETE FROM dm_conversations WHERE record_key = ?1",
            params![record_key],
        ).map_err(|e| DmStoreError::Storage(e.to_string()))?;
        Ok(())
    }

    fn dm_get_session_meta(
        &self, record_key: &str,
    ) -> Result<Option<DmSessionMeta>, DmStoreError> {
        let conn = self.conn();
        let result = conn.query_row(
            "SELECT my_subkey, initiator_pseudo, initiator_pub_key,
                    is_group, slot_seed_hex, participants_json
             FROM dm_conversations WHERE record_key = ?1",
            params![record_key],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                ))
            },
        );
        match result {
            Ok((subkey, pseudo, pub_key, is_group, seed_hex, participants_json)) => {
                let seed_bytes = hex::decode(&seed_hex)
                    .map_err(|e| DmStoreError::InvalidData(
                        format!("slot_seed_hex: {e}"),
                    ))?;
                let slot_seed: [u8; 32] = seed_bytes.try_into()
                    .map_err(|_| DmStoreError::InvalidData(
                        "slot_seed must be 32 bytes".into(),
                    ))?;
                let my_subkey = u32::try_from(subkey).unwrap_or(0);

                // Compute peer_public_key from participants list:
                // find the participant whose subkey differs from ours
                // and who has a non-empty public key.
                let participants: Vec<DmParticipant> =
                    serde_json::from_str(&participants_json).unwrap_or_default();
                let peer_public_key = participants.iter()
                    .find(|p| p.subkey != my_subkey && !p.public_key.is_empty())
                    .map(|p| p.public_key.clone())
                    .unwrap_or_default();

                Ok(Some(DmSessionMeta {
                    my_subkey,
                    initiator_pseudonym: pseudo,
                    initiator_public_key: pub_key,
                    peer_public_key,
                    is_group: is_group != 0,
                    slot_seed,
                }))
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(DmStoreError::Storage(e.to_string())),
        }
    }

    fn dm_next_sequence(
        &self, record_key: &str, sender_pseudonym: &str,
    ) -> Result<u64, DmStoreError> {
        let conn = self.conn();
        let max: Option<i64> = conn.query_row(
            "SELECT MAX(sequence) FROM dm_messages
             WHERE record_key = ?1 AND sender_pseudonym = ?2",
            params![record_key, sender_pseudonym],
            |row| row.get(0),
        ).map_err(|e| DmStoreError::Storage(e.to_string()))?;
        Ok(max.map(|n| u64::try_from(n).unwrap_or(0) + 1).unwrap_or(1))
    }

    fn dm_persist_message(&self, msg: DmMessageInsert) -> Result<(), DmStoreError> {
        let ct = self.encrypt_entry(msg.body.as_bytes())
            .map_err(|e| DmStoreError::Storage(format!("entry encrypt: {e}")))?;
        let conn = self.conn();
        conn.execute(
            "INSERT OR IGNORE INTO dm_messages
                (record_key, sender_pseudonym, body, timestamp_secs,
                 sequence, mek_generation, is_self)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                msg.record_key,
                msg.sender_pseudonym,
                ct,
                i64::try_from(msg.timestamp_ms).unwrap_or(i64::MAX),
                i64::try_from(msg.sequence).unwrap_or(i64::MAX),
                i64::try_from(msg.mek_generation).unwrap_or(i64::MAX),
                i64::from(msg.is_self),
            ],
        ).map_err(|e| DmStoreError::Storage(e.to_string()))?;
        conn.execute(
            "UPDATE dm_conversations SET last_message_at = ?1
             WHERE record_key = ?2",
            params![i64::try_from(msg.timestamp_ms).unwrap_or(i64::MAX), msg.record_key],
        ).map_err(|e| DmStoreError::Storage(e.to_string()))?;
        Ok(())
    }

    fn dm_oldest_recent_ts(
        &self, record_key: &str, lookback: i64,
    ) -> Result<Option<i64>, DmStoreError> {
        let conn = self.conn();
        let ts: Option<i64> = conn.query_row(
            "SELECT MIN(timestamp_secs) FROM (
                SELECT timestamp_secs FROM dm_messages
                WHERE record_key = ?1
                ORDER BY sequence DESC LIMIT ?2
             )",
            params![record_key, lookback],
            |row| row.get(0),
        ).map_err(|e| DmStoreError::Storage(e.to_string()))?;
        Ok(ts)
    }

    fn dm_update_mek_generation(
        &self, record_key: &str, new_gen: u32,
    ) -> Result<(), DmStoreError> {
        let conn = self.conn();
        conn.execute(
            "UPDATE dm_conversations SET mek_generation = ?1
             WHERE record_key = ?2",
            params![i64::from(new_gen), record_key],
        ).map_err(|e| DmStoreError::Storage(e.to_string()))?;
        Ok(())
    }

    fn dm_load_invite_meta(
        &self, record_key: &str,
    ) -> Result<Option<DmInviteMeta>, DmStoreError> {
        let conn = self.conn();
        let result = conn.query_row(
            "SELECT initiator_pub_key, my_subkey, mek_generation,
                    is_group, wrapped_mek_blob, participants_json
             FROM dm_conversations WHERE record_key = ?1",
            params![record_key],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, Option<Vec<u8>>>(4)?,
                    row.get::<_, String>(5)?,
                ))
            },
        );
        match result {
            Ok((pub_key, subkey, gen, is_group, blob, pj)) => {
                let participants: Vec<DmParticipant> =
                    serde_json::from_str(&pj).unwrap_or_default();
                Ok(Some(DmInviteMeta {
                    initiator_public_key: pub_key,
                    my_subkey: u32::try_from(subkey).unwrap_or(0),
                    mek_generation: u64::try_from(gen).unwrap_or(0),
                    is_group: is_group != 0,
                    wrapped_mek_blob: blob,
                    participants,
                }))
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(DmStoreError::Storage(e.to_string())),
        }
    }

    fn is_dm_hash_known(&self, hash: &[u8; 32]) -> Result<bool, DmStoreError> {
        let conn = self.conn();
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM dm_message_hashes WHERE hash = ?1",
            params![hash.as_slice()],
            |row| row.get(0),
        ).map_err(|e| DmStoreError::Storage(e.to_string()))?;
        Ok(count > 0)
    }

    fn store_dm_hash(
        &self, hash: &[u8; 32], record_key: &str,
    ) -> Result<(), DmStoreError> {
        let conn = self.conn();
        conn.execute(
            "INSERT OR IGNORE INTO dm_message_hashes (hash, record_key, created_at)
             VALUES (?1, ?2, ?3)",
            params![hash.as_slice(), record_key, crate::vault::schema::timestamp_secs()],
        ).map_err(|e| DmStoreError::Storage(e.to_string()))?;
        Ok(())
    }

    fn sweep_dm_hashes(&self, max_age_secs: i64) -> Result<u64, DmStoreError> {
        let conn = self.conn();
        let cutoff = crate::vault::schema::timestamp_secs() - max_age_secs;
        let deleted = conn.execute(
            "DELETE FROM dm_message_hashes WHERE created_at < ?1",
            params![cutoff],
        ).map_err(|e| DmStoreError::Storage(e.to_string()))?;
        Ok(deleted as u64)
    }
}
