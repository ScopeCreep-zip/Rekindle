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
mod tests {
    use super::*;

    async fn in_memory_store_with_schema() -> SqliteDmStore {
        let conn = tokio_rusqlite::Connection::open_in_memory().await.unwrap();
        conn.call(|c| -> rusqlite::Result<()> {
            c.execute_batch(
                "CREATE TABLE dms (
                    owner_key TEXT NOT NULL,
                    record_key TEXT NOT NULL,
                    is_group INTEGER NOT NULL,
                    initiator_public_key TEXT NOT NULL,
                    initiator_pseudonym TEXT NOT NULL,
                    my_subkey INTEGER NOT NULL,
                    participants_json TEXT NOT NULL,
                    slot_seed_hex TEXT NOT NULL,
                    wrapped_mek_blob BLOB,
                    mek_generation INTEGER NOT NULL,
                    created_at INTEGER NOT NULL,
                    last_message_at INTEGER,
                    PRIMARY KEY (owner_key, record_key)
                );
                CREATE TABLE dm_messages (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    owner_key TEXT NOT NULL,
                    record_key TEXT NOT NULL,
                    sender_pseudonym TEXT NOT NULL,
                    body TEXT NOT NULL,
                    timestamp INTEGER NOT NULL,
                    sequence INTEGER NOT NULL,
                    mek_generation INTEGER NOT NULL
                );",
            )?;
            Ok(())
        })
        .await
        .unwrap();
        SqliteDmStore::new(conn)
    }

    fn sample_invite() -> DmInvitePending {
        DmInvitePending {
            record_key: "rec123".into(),
            is_group: false,
            initiator_public_key: "pk_initiator".into(),
            initiator_pseudonym: "alice".into(),
            my_subkey: 1,
            participants: vec![],
            mek_generation: 0,
            // 32-byte slot seed as 64 hex chars; required by get_session_meta
            // and the production slot-keypair derivation path.
            slot_seed_hex: "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef"
                .into(),
            wrapped_mek_blob: None,
            created_at: 100,
        }
    }

    #[tokio::test]
    async fn empty_owner_key_rejected_on_persist() {
        let store = in_memory_store_with_schema().await;
        let err = store
            .persist_invite_pending("", sample_invite())
            .await
            .unwrap_err();
        assert!(matches!(err, DmError::InvalidInput(_)));
    }

    #[tokio::test]
    async fn persist_then_list_returns_inserted_row() {
        let store = in_memory_store_with_schema().await;
        store
            .persist_invite_pending("owner1", sample_invite())
            .await
            .unwrap();
        let list = store.list_conversations("owner1").await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].record_key, "rec123");
        assert_eq!(list[0].initiator_pseudonym, "alice");
        assert!(!list[0].is_group);
    }

    #[tokio::test]
    async fn list_scopes_by_owner_key() {
        let store = in_memory_store_with_schema().await;
        store
            .persist_invite_pending("ownerA", sample_invite())
            .await
            .unwrap();
        let mut bob_invite = sample_invite();
        bob_invite.record_key = "rec456".into();
        store
            .persist_invite_pending("ownerB", bob_invite)
            .await
            .unwrap();
        assert_eq!(store.list_conversations("ownerA").await.unwrap().len(), 1);
        assert_eq!(store.list_conversations("ownerB").await.unwrap().len(), 1);
        assert_eq!(store.list_conversations("ownerC").await.unwrap().len(), 0);
    }

    #[tokio::test]
    async fn persist_is_idempotent_on_conflict() {
        let store = in_memory_store_with_schema().await;
        store
            .persist_invite_pending("owner1", sample_invite())
            .await
            .unwrap();
        // Second insert with same (owner_key, record_key) is no-op.
        store
            .persist_invite_pending("owner1", sample_invite())
            .await
            .unwrap();
        assert_eq!(store.list_conversations("owner1").await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn decline_removes_row() {
        let store = in_memory_store_with_schema().await;
        store
            .persist_invite_pending("owner1", sample_invite())
            .await
            .unwrap();
        store.decline_invite("owner1", "rec123").await.unwrap();
        assert_eq!(store.list_conversations("owner1").await.unwrap().len(), 0);
    }

    #[tokio::test]
    async fn load_messages_returns_oldest_first() {
        let store = in_memory_store_with_schema().await;
        store
            .persist_invite_pending("owner1", sample_invite())
            .await
            .unwrap();
        store
            .conn
            .call(|c| -> rusqlite::Result<()> {
                c.execute(
                    "INSERT INTO dm_messages (owner_key, record_key, sender_pseudonym, body, timestamp, sequence, mek_generation) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    rusqlite::params!["owner1", "rec123", "alice", "hello", 200_i64, 1_i64, 0_i64],
                )?;
                c.execute(
                    "INSERT INTO dm_messages (owner_key, record_key, sender_pseudonym, body, timestamp, sequence, mek_generation) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    rusqlite::params!["owner1", "rec123", "bob", "hi back", 300_i64, 2_i64, 0_i64],
                )?;
                Ok(())
            })
            .await
            .unwrap();
        let msgs = store.load_messages("owner1", "rec123", 10).await.unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].body, "hello");
        assert_eq!(msgs[1].body, "hi back");
    }

    #[tokio::test]
    async fn get_session_meta_returns_persisted_fields() {
        let store = in_memory_store_with_schema().await;
        store
            .persist_invite_pending("owner1", sample_invite())
            .await
            .unwrap();
        let meta = store
            .get_session_meta("owner1", "rec123")
            .await
            .unwrap()
            .expect("session meta should exist");
        assert_eq!(meta.my_subkey, 1);
        assert_eq!(meta.initiator_pseudonym, "alice");
        assert_eq!(meta.initiator_public_key, "pk_initiator");
        assert!(!meta.is_group);
        assert_eq!(meta.slot_seed.len(), 32);
    }

    #[tokio::test]
    async fn get_session_meta_returns_none_for_missing_row() {
        let store = in_memory_store_with_schema().await;
        let meta = store
            .get_session_meta("owner1", "doesnt-exist")
            .await
            .unwrap();
        assert!(meta.is_none());
    }

    #[tokio::test]
    async fn get_session_meta_rejects_invalid_slot_seed_hex() {
        let store = in_memory_store_with_schema().await;
        store
            .conn
            .call(|c| -> rusqlite::Result<()> {
                c.execute(
                    "INSERT INTO dms (owner_key, record_key, is_group, initiator_public_key,
                        initiator_pseudonym, my_subkey, participants_json, slot_seed_hex,
                        wrapped_mek_blob, mek_generation, created_at, last_message_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, NULL, ?9, ?10, NULL)",
                    rusqlite::params![
                        "owner1", "badrec", 0_i64, "pk", "alice", 1_i64, "[]", "tooshort", 0_i64,
                        100_i64,
                    ],
                )?;
                Ok(())
            })
            .await
            .unwrap();
        let err = store
            .get_session_meta("owner1", "badrec")
            .await
            .unwrap_err();
        assert!(matches!(err, DmError::InvalidInput(_)));
    }

    #[tokio::test]
    async fn next_sequence_starts_at_one_for_empty_history() {
        let store = in_memory_store_with_schema().await;
        let seq = store
            .next_sequence_for_sender("owner1", "rec123", "alice")
            .await
            .unwrap();
        assert_eq!(seq, 1);
    }

    #[tokio::test]
    async fn next_sequence_increments_per_sender() {
        let store = in_memory_store_with_schema().await;
        store
            .conn
            .call(|c| -> rusqlite::Result<()> {
                c.execute(
                    "INSERT INTO dm_messages (owner_key, record_key, sender_pseudonym, body, timestamp, sequence, mek_generation) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    rusqlite::params!["owner1", "rec123", "alice", "m1", 100_i64, 1_i64, 0_i64],
                )?;
                c.execute(
                    "INSERT INTO dm_messages (owner_key, record_key, sender_pseudonym, body, timestamp, sequence, mek_generation) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    rusqlite::params!["owner1", "rec123", "alice", "m2", 200_i64, 5_i64, 0_i64],
                )?;
                c.execute(
                    "INSERT INTO dm_messages (owner_key, record_key, sender_pseudonym, body, timestamp, sequence, mek_generation) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    rusqlite::params!["owner1", "rec123", "bob", "m1", 150_i64, 1_i64, 0_i64],
                )?;
                Ok(())
            })
            .await
            .unwrap();
        let alice = store
            .next_sequence_for_sender("owner1", "rec123", "alice")
            .await
            .unwrap();
        let bob = store
            .next_sequence_for_sender("owner1", "rec123", "bob")
            .await
            .unwrap();
        assert_eq!(alice, 6, "next after max-seen alice seq 5");
        assert_eq!(bob, 2, "next after max-seen bob seq 1");
    }

    #[tokio::test]
    async fn persist_message_inserts_and_updates_last_message_at() {
        let store = in_memory_store_with_schema().await;
        store
            .persist_invite_pending("owner1", sample_invite())
            .await
            .unwrap();
        let insert = DmMessageInsert {
            record_key: "rec123".into(),
            sender_pseudonym: "alice".into(),
            body: "hi".into(),
            timestamp_secs: 555,
            sequence: 1,
            mek_generation: 0,
        };
        store.persist_message("owner1", insert).await.unwrap();
        let msgs = store.load_messages("owner1", "rec123", 10).await.unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].body, "hi");
        let convos = store.list_conversations("owner1").await.unwrap();
        assert_eq!(convos[0].last_message_at, Some(555));
    }

    #[tokio::test]
    async fn oldest_recent_message_ts_returns_min_within_lookback() {
        let store = in_memory_store_with_schema().await;
        store
            .conn
            .call(|c| -> rusqlite::Result<()> {
                for seq in 1..=5_i64 {
                    c.execute(
                        "INSERT INTO dm_messages (owner_key, record_key, sender_pseudonym, body, timestamp, sequence, mek_generation) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                        rusqlite::params![
                            "owner1",
                            "rec123",
                            "alice",
                            format!("m{seq}"),
                            seq * 100,
                            seq,
                            0_i64,
                        ],
                    )?;
                }
                Ok(())
            })
            .await
            .unwrap();
        let ts = store
            .oldest_recent_message_ts("owner1", "rec123", 3)
            .await
            .unwrap();
        assert_eq!(ts, Some(300));
        let ts_all = store
            .oldest_recent_message_ts("owner1", "rec123", 100)
            .await
            .unwrap();
        assert_eq!(ts_all, Some(100));
    }

    #[tokio::test]
    async fn update_mek_generation_persists() {
        let store = in_memory_store_with_schema().await;
        store
            .persist_invite_pending("owner1", sample_invite())
            .await
            .unwrap();
        store
            .update_mek_generation("owner1", "rec123", 42)
            .await
            .unwrap();
        let convos = store.list_conversations("owner1").await.unwrap();
        assert_eq!(convos[0].mek_generation, 42);
    }

    #[tokio::test]
    async fn load_invite_meta_returns_full_fields() {
        let store = in_memory_store_with_schema().await;
        let mut invite = sample_invite();
        invite.is_group = true;
        invite.wrapped_mek_blob = Some(vec![1, 2, 3, 4]);
        invite.mek_generation = 7;
        invite.participants = vec![GroupDmParticipant {
            pseudonym: "bob".into(),
            subkey: 0,
            public_key: "pk_bob".into(),
        }];
        store
            .persist_invite_pending("owner1", invite)
            .await
            .unwrap();
        let meta = store
            .load_invite_meta("owner1", "rec123")
            .await
            .unwrap()
            .expect("invite meta should exist");
        assert!(meta.is_group);
        assert_eq!(meta.wrapped_mek_blob.as_deref(), Some(&[1u8, 2, 3, 4][..]));
        assert_eq!(meta.mek_generation, 7);
        assert_eq!(meta.my_subkey, 1);
        assert_eq!(meta.participants.len(), 1);
        assert_eq!(meta.participants[0].pseudonym, "bob");
    }

    #[tokio::test]
    async fn load_invite_meta_returns_none_for_missing_row() {
        let store = in_memory_store_with_schema().await;
        let meta = store.load_invite_meta("owner1", "missing").await.unwrap();
        assert!(meta.is_none());
    }
}
