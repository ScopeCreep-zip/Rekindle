//! W16.9 — SQLite-backed [`EnvelopeStore`] for the Tauri shell.
//!
//! Reuses the existing [`crate::db::DbPool`] (a `tokio_rusqlite::Connection`)
//! so the entire reliability layer (retry queue + dedup + active call
//! state) lives in the same database as chat history, friend records,
//! and signal sessions. The CLI and daemon use the JSON-on-disk impl
//! provided by transport (`JsonEnvelopeStore`); this is the Tauri-only
//! impl that delegates to rusqlite.

use std::sync::Arc;

use async_trait::async_trait;
use rekindle_transport::envelope_store::{
    EnvelopeKind, EnvelopeStore, PendingEnvelope, PersistedCallState, StoreError,
};

use crate::db::DbPool;
use crate::db_helpers::db_call;

/// SQLite-backed [`EnvelopeStore`]. Cloned cheaply (single `Arc<DbPool>`
/// inside).
pub struct SqliteEnvelopeStore {
    pool: Arc<DbPool>,
}

impl SqliteEnvelopeStore {
    pub fn new(pool: Arc<DbPool>) -> Self {
        Self { pool }
    }

    /// Convenience: wrap as the trait object expected by transport.
    pub fn into_dyn(self) -> Arc<dyn EnvelopeStore> {
        Arc::new(self)
    }
}

fn map_db_err(reason: String) -> StoreError {
    StoreError::Other(reason)
}

#[async_trait]
impl EnvelopeStore for SqliteEnvelopeStore {
    async fn enqueue(&self, env: PendingEnvelope) -> Result<i64, StoreError> {
        let kind_str = env.kind.as_str().to_string();
        let owner = env.owner_key.clone();
        let recipient = env.recipient_key.clone();
        let correlation = env.correlation_id.clone();
        let payload = env.payload.clone();
        let seq = i64::try_from(env.seq).unwrap_or(i64::MAX);
        let created_at = i64::try_from(env.created_at_ms).unwrap_or(i64::MAX);
        let next_retry = i64::try_from(env.next_retry_at_ms).unwrap_or(i64::MAX);
        let retry_count = i64::from(env.retry_count);
        let max_retries = i64::from(env.max_retries);

        db_call(&self.pool, move |conn| {
            conn.execute(
                "INSERT INTO pending_envelopes
                    (owner_key, recipient_key, envelope_kind, seq, correlation_id,
                     payload, created_at_ms, next_retry_at_ms, retry_count,
                     max_retries, last_error)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, NULL)",
                rusqlite::params![
                    owner, recipient, kind_str, seq, correlation,
                    payload, created_at, next_retry, retry_count, max_retries,
                ],
            )?;
            Ok(conn.last_insert_rowid())
        })
        .await
        .map_err(map_db_err)
    }

    async fn load_eligible(
        &self,
        owner_key: &str,
        now_ms: u64,
        limit: usize,
    ) -> Result<Vec<PendingEnvelope>, StoreError> {
        let owner = owner_key.to_string();
        let now = i64::try_from(now_ms).unwrap_or(i64::MAX);
        let lim = i64::try_from(limit).unwrap_or(64);

        db_call(&self.pool, move |conn| {
            let mut stmt = conn.prepare(
                "SELECT id, recipient_key, envelope_kind, seq, correlation_id,
                        payload, created_at_ms, next_retry_at_ms, retry_count,
                        max_retries, last_error
                 FROM pending_envelopes
                 WHERE owner_key = ?1 AND next_retry_at_ms <= ?2
                 ORDER BY next_retry_at_ms ASC
                 LIMIT ?3",
            )?;
            let rows = stmt.query_map(rusqlite::params![owner, now, lim], |row| {
                let kind_str: String = row.get(2)?;
                let kind = EnvelopeKind::from_wire(&kind_str)
                    .ok_or_else(|| rusqlite::Error::FromSqlConversionFailure(
                        2,
                        rusqlite::types::Type::Text,
                        format!("unknown envelope_kind: {kind_str}").into(),
                    ))?;
                let seq_i: i64 = row.get(3)?;
                let created_at_i: i64 = row.get(6)?;
                let next_retry_i: i64 = row.get(7)?;
                let retry_count_i: i64 = row.get(8)?;
                let max_retries_i: i64 = row.get(9)?;
                Ok(PendingEnvelope {
                    id: row.get(0)?,
                    owner_key: String::new(), // filled below from query owner
                    recipient_key: row.get(1)?,
                    kind,
                    seq: u64::try_from(seq_i).unwrap_or(0),
                    correlation_id: row.get::<_, Option<String>>(4)?,
                    payload: row.get(5)?,
                    created_at_ms: u64::try_from(created_at_i).unwrap_or(0),
                    next_retry_at_ms: u64::try_from(next_retry_i).unwrap_or(0),
                    retry_count: u32::try_from(retry_count_i).unwrap_or(0),
                    max_retries: u32::try_from(max_retries_i).unwrap_or(0),
                    last_error: row.get::<_, Option<String>>(10)?,
                })
            })?;

            let mut out = Vec::new();
            for r in rows {
                let mut env = r?;
                env.owner_key.clone_from(&owner);
                out.push(env);
            }
            Ok(out)
        })
        .await
        .map_err(map_db_err)
    }

    async fn mark_delivered(&self, id: i64) -> Result<(), StoreError> {
        db_call(&self.pool, move |conn| {
            let n = conn.execute(
                "DELETE FROM pending_envelopes WHERE id = ?1",
                rusqlite::params![id],
            )?;
            if n == 0 {
                return Err(rusqlite::Error::QueryReturnedNoRows);
            }
            Ok(())
        })
        .await
        .map_err(|e| {
            // Map "no rows" specifically to NotFound; everything else
            // is a generic Io error.
            if e.contains("Query returned no rows") {
                StoreError::NotFound(id)
            } else {
                StoreError::Other(e)
            }
        })
    }

    async fn mark_retry(
        &self,
        id: i64,
        retry_count: u32,
        next_retry_at_ms: u64,
        last_error: &str,
    ) -> Result<(), StoreError> {
        let count = i64::from(retry_count);
        let next = i64::try_from(next_retry_at_ms).unwrap_or(i64::MAX);
        let err = last_error.to_string();
        db_call(&self.pool, move |conn| {
            let n = conn.execute(
                "UPDATE pending_envelopes
                 SET retry_count = ?1, next_retry_at_ms = ?2, last_error = ?3
                 WHERE id = ?4",
                rusqlite::params![count, next, err, id],
            )?;
            if n == 0 {
                return Err(rusqlite::Error::QueryReturnedNoRows);
            }
            Ok(())
        })
        .await
        .map_err(|e| {
            if e.contains("Query returned no rows") {
                StoreError::NotFound(id)
            } else {
                StoreError::Other(e)
            }
        })
    }

    async fn mark_dead(&self, id: i64) -> Result<(), StoreError> {
        // Same semantic as delivered (delete the row); caller emits the
        // EnvelopeDeliveryFailed notification before calling this.
        self.mark_delivered(id).await
    }

    async fn cancel_by_correlation(
        &self,
        correlation_id: &str,
    ) -> Result<usize, StoreError> {
        let cid = correlation_id.to_string();
        db_call(&self.pool, move |conn| {
            let n = conn.execute(
                "DELETE FROM pending_envelopes WHERE correlation_id = ?1",
                rusqlite::params![cid],
            )?;
            Ok(n)
        })
        .await
        .map_err(map_db_err)
    }

    async fn next_outbound_seq(
        &self,
        owner_key: &str,
        recipient_key: &str,
        kind: EnvelopeKind,
        correlation_id: &str,
    ) -> Result<u64, StoreError> {
        let owner = owner_key.to_string();
        let recipient = recipient_key.to_string();
        let kind_str = kind.as_str().to_string();
        let cid = correlation_id.to_string();

        db_call(&self.pool, move |conn| {
            let tx = conn.transaction()?;
            let current: Option<i64> = tx
                .query_row(
                    "SELECT next_seq FROM outbound_seqs
                     WHERE owner_key = ?1 AND recipient_key = ?2
                       AND envelope_kind = ?3 AND correlation_id = ?4",
                    rusqlite::params![owner, recipient, kind_str, cid],
                    |row| row.get(0),
                )
                .ok();
            let next = current.unwrap_or(0) + 1;
            tx.execute(
                "INSERT OR REPLACE INTO outbound_seqs
                    (owner_key, recipient_key, envelope_kind, correlation_id, next_seq)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![owner, recipient, kind_str, cid, next],
            )?;
            tx.commit()?;
            Ok(u64::try_from(next).unwrap_or(0))
        })
        .await
        .map_err(map_db_err)
    }

    async fn record_inbound_seq(
        &self,
        owner_key: &str,
        sender_key: &str,
        kind: EnvelopeKind,
        correlation_id: &str,
        seq: u64,
        now_ms: u64,
    ) -> Result<(), StoreError> {
        let owner = owner_key.to_string();
        let sender = sender_key.to_string();
        let kind_str = kind.as_str().to_string();
        let cid = correlation_id.to_string();
        let seq_i = i64::try_from(seq).unwrap_or(i64::MAX);
        let now = i64::try_from(now_ms).unwrap_or(i64::MAX);

        db_call(&self.pool, move |conn| {
            conn.execute(
                "INSERT OR REPLACE INTO seen_envelopes
                    (owner_key, sender_key, envelope_kind, correlation_id,
                     last_seq, last_seen_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![owner, sender, kind_str, cid, seq_i, now],
            )?;
            Ok(())
        })
        .await
        .map_err(map_db_err)
    }

    async fn get_last_inbound_seq(
        &self,
        owner_key: &str,
        sender_key: &str,
        kind: EnvelopeKind,
        correlation_id: &str,
    ) -> Result<Option<u64>, StoreError> {
        let owner = owner_key.to_string();
        let sender = sender_key.to_string();
        let kind_str = kind.as_str().to_string();
        let cid = correlation_id.to_string();

        db_call(&self.pool, move |conn| {
            let result: Option<i64> = conn
                .query_row(
                    "SELECT last_seq FROM seen_envelopes
                     WHERE owner_key = ?1 AND sender_key = ?2
                       AND envelope_kind = ?3 AND correlation_id = ?4",
                    rusqlite::params![owner, sender, kind_str, cid],
                    |row| row.get(0),
                )
                .ok();
            Ok(result.map(|s| u64::try_from(s).unwrap_or(0)))
        })
        .await
        .map_err(map_db_err)
    }

    async fn save_active_call(&self, state: PersistedCallState) -> Result<(), StoreError> {
        let owner = state.owner_key.clone();
        let call_id = state.call_id.clone();
        let peer = state.peer_pubkey.clone();
        let kind = state.kind.clone();
        let status = state.status.clone();
        let expires = i64::try_from(state.expires_at_ms).unwrap_or(i64::MAX);
        let my_secret = state.my_x25519_secret.clone();
        let peer_pub = state.peer_x25519_pub.clone();
        let participants = serde_json::to_string(&state.group_participants)
            .map_err(|e| StoreError::Serialize(e.to_string()))?;
        let inserted = i64::try_from(state.inserted_at_ms).unwrap_or(i64::MAX);

        db_call(&self.pool, move |conn| {
            conn.execute(
                "INSERT OR REPLACE INTO active_call_states
                    (owner_key, call_id, peer_pubkey, kind, status,
                     expires_at_ms, my_x25519_secret, peer_x25519_pub,
                     group_participants, inserted_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                rusqlite::params![
                    owner, call_id, peer, kind, status,
                    expires, my_secret, peer_pub, participants, inserted,
                ],
            )?;
            Ok(())
        })
        .await
        .map_err(map_db_err)
    }

    async fn delete_active_call(
        &self,
        owner_key: &str,
        call_id: &str,
    ) -> Result<(), StoreError> {
        let owner = owner_key.to_string();
        let cid = call_id.to_string();
        db_call(&self.pool, move |conn| {
            conn.execute(
                "DELETE FROM active_call_states WHERE owner_key = ?1 AND call_id = ?2",
                rusqlite::params![owner, cid],
            )?;
            Ok(())
        })
        .await
        .map_err(map_db_err)
    }

    async fn load_active_calls(
        &self,
        owner_key: &str,
    ) -> Result<Vec<PersistedCallState>, StoreError> {
        let owner = owner_key.to_string();
        db_call(&self.pool, move |conn| {
            let mut stmt = conn.prepare(
                "SELECT call_id, peer_pubkey, kind, status, expires_at_ms,
                        my_x25519_secret, peer_x25519_pub, group_participants,
                        inserted_at_ms
                 FROM active_call_states
                 WHERE owner_key = ?1",
            )?;
            let rows = stmt.query_map(rusqlite::params![owner], |row| {
                let expires_i: i64 = row.get(4)?;
                let inserted_i: i64 = row.get(8)?;
                let participants_str: String = row.get(7)?;
                let group_participants: Vec<String> = serde_json::from_str(&participants_str)
                    .unwrap_or_default();
                Ok(PersistedCallState {
                    owner_key: String::new(), // filled below
                    call_id: row.get(0)?,
                    peer_pubkey: row.get(1)?,
                    kind: row.get(2)?,
                    status: row.get(3)?,
                    expires_at_ms: u64::try_from(expires_i).unwrap_or(0),
                    my_x25519_secret: row.get::<_, Option<Vec<u8>>>(5)?,
                    peer_x25519_pub: row.get::<_, Option<Vec<u8>>>(6)?,
                    group_participants,
                    inserted_at_ms: u64::try_from(inserted_i).unwrap_or(0),
                })
            })?;

            let mut out = Vec::new();
            for r in rows {
                let mut state = r?;
                state.owner_key.clone_from(&owner);
                out.push(state);
            }
            Ok(out)
        })
        .await
        .map_err(map_db_err)
    }
}
