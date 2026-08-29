//! Phase 13 — SQLite-backed `DmStore` impl.
//!
//! Concrete impl of the `DmStore` trait (defined in `super`) against
//! the Rekindle SQLite schema (`dms` + `dm_messages` tables, defined in
//! `src-tauri/migrations/001_init.sql`). Wraps a shared
//! `tokio_rusqlite::Connection` (the same pool type src-tauri holds in
//! `DbPool`).

use async_trait::async_trait;
use tokio_rusqlite::Connection;

use crate::error::DmError;
use crate::invite::GroupDmParticipant;

use super::{
    DmConversation, DmInviteMeta, DmInvitePending, DmMessageInsert, DmMessageRecord, DmSessionMeta,
    DmStore,
};

/// SQLite-backed `DmStore`. Wraps a shared `tokio_rusqlite::Connection`
/// (the same pool type src-tauri holds in `DbPool`). The schema is
/// defined in `src-tauri/migrations/001_init.sql` (`dms` and
/// `dm_messages` tables) — this impl assumes those tables exist.
pub struct SqliteDmStore {
    conn: Connection,
}

impl SqliteDmStore {
    #[must_use]
    pub fn new(conn: Connection) -> Self {
        Self { conn }
    }
}

#[async_trait]
impl DmStore for SqliteDmStore {
    async fn persist_invite_pending(
        &self,
        owner_key: &str,
        invite: DmInvitePending,
    ) -> Result<(), DmError> {
        if owner_key.is_empty() {
            return Err(DmError::InvalidInput("empty owner_key".into()));
        }
        let owner = owner_key.to_string();
        let participants_json = serde_json::to_string(&invite.participants)
            .map_err(|e| DmError::Serialize(format!("dm participants: {e}")))?;
        let group_flag = i64::from(invite.is_group);
        let my_subkey_i = i64::from(invite.my_subkey);
        let gen_i = i64::from(invite.mek_generation);
        self.conn
            .call(move |conn| -> rusqlite::Result<()> {
                conn.execute(
                    "INSERT INTO dms
                        (owner_key, record_key, is_group, initiator_public_key, initiator_pseudonym,
                         my_subkey, participants_json, slot_seed_hex, wrapped_mek_blob,
                         mek_generation, created_at, last_message_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, NULL)
                     ON CONFLICT(owner_key, record_key) DO NOTHING",
                    rusqlite::params![
                        owner,
                        invite.record_key,
                        group_flag,
                        invite.initiator_public_key,
                        invite.initiator_pseudonym,
                        my_subkey_i,
                        participants_json,
                        invite.slot_seed_hex,
                        invite.wrapped_mek_blob,
                        gen_i,
                        invite.created_at,
                    ],
                )?;
                Ok(())
            })
            .await
            .map_err(|e| DmError::Sqlite(e.to_string()))
    }

    async fn list_conversations(&self, owner_key: &str) -> Result<Vec<DmConversation>, DmError> {
        if owner_key.is_empty() {
            return Ok(Vec::new());
        }
        let owner = owner_key.to_string();
        self.conn
            .call(move |conn| -> rusqlite::Result<Vec<DmConversation>> {
                let mut stmt = conn.prepare(
                    "SELECT record_key, is_group, initiator_public_key, initiator_pseudonym,
                            my_subkey, participants_json, mek_generation, created_at, last_message_at
                     FROM dms WHERE owner_key = ?1 ORDER BY COALESCE(last_message_at, created_at) DESC",
                )?;
                let rows = stmt
                    .query_map(rusqlite::params![owner], |row| {
                        let participants_json: String = row.get(5)?;
                        let participants: Vec<GroupDmParticipant> =
                            serde_json::from_str(&participants_json).unwrap_or_default();
                        Ok(DmConversation {
                            record_key: row.get(0)?,
                            is_group: {
                                let n: i64 = row.get(1)?;
                                n != 0
                            },
                            initiator_public_key: row.get(2)?,
                            initiator_pseudonym: row.get(3)?,
                            my_subkey: {
                                let n: i64 = row.get(4)?;
                                u32::try_from(n).unwrap_or(0)
                            },
                            participants,
                            mek_generation: {
                                let n: i64 = row.get(6)?;
                                u32::try_from(n).unwrap_or(0)
                            },
                            created_at: row.get(7)?,
                            last_message_at: row.get(8)?,
                        })
                    })?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .await
            .map_err(|e| DmError::Sqlite(e.to_string()))
    }

    async fn load_messages(
        &self,
        owner_key: &str,
        record_key: &str,
        limit: i64,
    ) -> Result<Vec<DmMessageRecord>, DmError> {
        if owner_key.is_empty() {
            return Ok(Vec::new());
        }
        let owner = owner_key.to_string();
        let record = record_key.to_string();
        self.conn
            .call(move |conn| -> rusqlite::Result<Vec<DmMessageRecord>> {
                let mut stmt = conn.prepare(
                    "SELECT id, sender_pseudonym, body, timestamp, sequence, mek_generation
                     FROM dm_messages
                     WHERE owner_key = ?1 AND record_key = ?2
                     ORDER BY timestamp DESC LIMIT ?3",
                )?;
                let rows = stmt.query_map(rusqlite::params![owner, record, limit], |row| {
                    Ok(DmMessageRecord {
                        id: row.get(0)?,
                        sender_pseudonym: row.get(1)?,
                        body: row.get(2)?,
                        timestamp: row.get(3)?,
                        sequence: row.get(4)?,
                        mek_generation: row.get(5)?,
                    })
                })?;
                let mut collected: Vec<DmMessageRecord> =
                    rows.collect::<rusqlite::Result<Vec<_>>>()?;
                collected.reverse(); // oldest-first for the UI scrollback
                Ok(collected)
            })
            .await
            .map_err(|e| DmError::Sqlite(e.to_string()))
    }

    async fn decline_invite(&self, owner_key: &str, record_key: &str) -> Result<(), DmError> {
        if owner_key.is_empty() {
            return Err(DmError::InvalidInput("empty owner_key".into()));
        }
        let owner = owner_key.to_string();
        let record = record_key.to_string();
        self.conn
            .call(move |conn| -> rusqlite::Result<()> {
                conn.execute(
                    "DELETE FROM dms WHERE owner_key = ?1 AND record_key = ?2",
                    rusqlite::params![owner, record],
                )?;
                Ok(())
            })
            .await
            .map_err(|e| DmError::Sqlite(e.to_string()))
    }

    async fn get_session_meta(
        &self,
        owner_key: &str,
        record_key: &str,
    ) -> Result<Option<DmSessionMeta>, DmError> {
        if owner_key.is_empty() {
            return Ok(None);
        }
        let owner = owner_key.to_string();
        let record = record_key.to_string();
        let row: Option<(i64, String, String, bool, String)> = self
            .conn
            .call(move |conn| -> rusqlite::Result<Option<(i64, String, String, bool, String)>> {
                let r = conn
                    .query_row(
                        "SELECT my_subkey, initiator_pseudonym, initiator_public_key, is_group, slot_seed_hex
                         FROM dms WHERE owner_key = ?1 AND record_key = ?2",
                        rusqlite::params![owner, record],
                        |row| {
                            Ok((
                                row.get::<_, i64>(0)?,
                                row.get::<_, String>(1)?,
                                row.get::<_, String>(2)?,
                                row.get::<_, i64>(3)? != 0,
                                row.get::<_, String>(4)?,
                            ))
                        },
                    )
                    .ok();
                Ok(r)
            })
            .await
            .map_err(|e| DmError::Sqlite(e.to_string()))?;
        let Some((my_subkey_i, pseudonym, initiator_pk, is_group, seed_hex)) = row else {
            return Ok(None);
        };
        let slot_seed_vec = hex::decode(&seed_hex)
            .map_err(|e| DmError::InvalidInput(format!("invalid slot seed hex: {e}")))?;
        let slot_seed: [u8; 32] = slot_seed_vec
            .try_into()
            .map_err(|_| DmError::InvalidInput("slot seed must be 32 bytes".into()))?;
        Ok(Some(DmSessionMeta {
            my_subkey: u32::try_from(my_subkey_i).unwrap_or(0),
            initiator_pseudonym: pseudonym,
            initiator_public_key: initiator_pk,
            is_group,
            slot_seed,
        }))
    }

    async fn next_sequence_for_sender(
        &self,
        owner_key: &str,
        record_key: &str,
        sender_pseudonym: &str,
    ) -> Result<u64, DmError> {
        if owner_key.is_empty() {
            return Ok(1);
        }
        let owner = owner_key.to_string();
        let record = record_key.to_string();
        let sender = sender_pseudonym.to_string();
        let prev_max: Option<i64> = self
            .conn
            .call(move |conn| -> rusqlite::Result<Option<i64>> {
                let prev: Option<i64> = conn
                    .query_row(
                        "SELECT MAX(sequence) FROM dm_messages
                         WHERE owner_key = ?1 AND record_key = ?2 AND sender_pseudonym = ?3",
                        rusqlite::params![owner, record, sender],
                        |row| row.get(0),
                    )
                    .ok()
                    .flatten();
                Ok(prev)
            })
            .await
            .map_err(|e| DmError::Sqlite(e.to_string()))?;
        Ok(u64::try_from(prev_max.unwrap_or(0)).unwrap_or(0) + 1)
    }

    async fn persist_message(&self, owner_key: &str, msg: DmMessageInsert) -> Result<(), DmError> {
        if owner_key.is_empty() {
            return Err(DmError::InvalidInput("empty owner_key".into()));
        }
        let owner = owner_key.to_string();
        let seq_i = i64::try_from(msg.sequence).unwrap_or(i64::MAX);
        let gen_i = i64::try_from(msg.mek_generation).unwrap_or(i64::MAX);
        let now_ms = msg.timestamp_secs; // also used for last_message_at
        self.conn
            .call(move |conn| -> rusqlite::Result<()> {
                conn.execute(
                    "INSERT INTO dm_messages
                        (owner_key, record_key, sender_pseudonym, body, timestamp,
                         sequence, mek_generation)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    rusqlite::params![
                        owner,
                        msg.record_key,
                        msg.sender_pseudonym,
                        msg.body,
                        msg.timestamp_secs,
                        seq_i,
                        gen_i,
                    ],
                )?;
                conn.execute(
                    "UPDATE dms SET last_message_at = ?3 WHERE owner_key = ?1 AND record_key = ?2",
                    rusqlite::params![owner, msg.record_key, now_ms],
                )?;
                Ok(())
            })
            .await
            .map_err(|e| DmError::Sqlite(e.to_string()))
    }

    async fn oldest_recent_message_ts(
        &self,
        owner_key: &str,
        record_key: &str,
        lookback: i64,
    ) -> Result<Option<i64>, DmError> {
        if owner_key.is_empty() {
            return Ok(None);
        }
        let owner = owner_key.to_string();
        let record = record_key.to_string();
        self.conn
            .call(move |conn| -> rusqlite::Result<Option<i64>> {
                let row: Option<i64> = conn
                    .query_row(
                        "SELECT MIN(timestamp) FROM dm_messages
                         WHERE owner_key = ?1 AND record_key = ?2
                           AND sequence > (
                             SELECT COALESCE(MAX(sequence), 0) - ?3 FROM dm_messages
                             WHERE owner_key = ?1 AND record_key = ?2
                           )",
                        rusqlite::params![owner, record, lookback],
                        |r| r.get(0),
                    )
                    .ok();
                Ok(row)
            })
            .await
            .map_err(|e| DmError::Sqlite(e.to_string()))
    }

    async fn update_mek_generation(
        &self,
        owner_key: &str,
        record_key: &str,
        new_generation: u32,
    ) -> Result<(), DmError> {
        if owner_key.is_empty() {
            return Err(DmError::InvalidInput("empty owner_key".into()));
        }
        let owner = owner_key.to_string();
        let record = record_key.to_string();
        let gen_i = i64::from(new_generation);
        self.conn
            .call(move |conn| -> rusqlite::Result<()> {
                conn.execute(
                    "UPDATE dms SET mek_generation = ?3
                     WHERE owner_key = ?1 AND record_key = ?2",
                    rusqlite::params![owner, record, gen_i],
                )?;
                Ok(())
            })
            .await
            .map_err(|e| DmError::Sqlite(e.to_string()))
    }

    async fn load_invite_meta(
        &self,
        owner_key: &str,
        record_key: &str,
    ) -> Result<Option<DmInviteMeta>, DmError> {
        if owner_key.is_empty() {
            return Ok(None);
        }
        let owner = owner_key.to_string();
        let record = record_key.to_string();
        let row: Option<(String, i64, i64, bool, Option<Vec<u8>>, String)> = self
            .conn
            .call(
                move |conn| -> rusqlite::Result<
                    Option<(String, i64, i64, bool, Option<Vec<u8>>, String)>,
                > {
                    let r = conn
                        .query_row(
                            "SELECT initiator_public_key, my_subkey, mek_generation, is_group,
                                    wrapped_mek_blob, participants_json
                             FROM dms WHERE owner_key = ?1 AND record_key = ?2",
                            rusqlite::params![owner, record],
                            |row| {
                                Ok((
                                    row.get::<_, String>(0)?,
                                    row.get::<_, i64>(1)?,
                                    row.get::<_, i64>(2)?,
                                    row.get::<_, i64>(3)? != 0,
                                    row.get::<_, Option<Vec<u8>>>(4)?,
                                    row.get::<_, String>(5)?,
                                ))
                            },
                        )
                        .ok();
                    Ok(r)
                },
            )
            .await
            .map_err(|e| DmError::Sqlite(e.to_string()))?;
        let Some((init_pk, my_subkey_i, mek_gen_i, is_group, wrapped, participants_json)) = row
        else {
            return Ok(None);
        };
        let participants: Vec<GroupDmParticipant> =
            serde_json::from_str(&participants_json).unwrap_or_default();
        Ok(Some(DmInviteMeta {
            initiator_public_key: init_pk,
            my_subkey: u32::try_from(my_subkey_i).unwrap_or(1),
            mek_generation: u64::try_from(mek_gen_i).unwrap_or(0),
            is_group,
            wrapped_mek_blob: wrapped,
            participants,
        }))
    }
}

#[cfg(test)]
#[path = "sqlite/tests.rs"]
mod tests;
