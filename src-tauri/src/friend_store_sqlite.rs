//! Track A.2 — SQLite-backed [`FriendStore`] for the Tauri shell.
//!
//! Reuses the existing [`crate::db::DbPool`] (a `tokio_rusqlite::Connection`)
//! so the receive-path friend authority lives in the same database as
//! chat history, friend records (`friends` table), and signal sessions.
//! The CLI and daemon will use a JSON-on-disk impl when their migration
//! lands; this is the Tauri-only impl that delegates to rusqlite.
//!
//! See `docs/security.md` and `.claude/plans/phase-2-dht-inbox-pivot.md`
//! Track A for the structural rationale (replaces in-memory `state.friends`
//! map that suffered a hydration race with the Veilid dispatch loop).
//!
//! # Hot path
//!
//! [`FriendStore::lookup_by_pubkey`] is called once per inbound envelope.
//! At realistic peak throughput (~100 envelopes/sec across active chat,
//! presence, and voice setup), ~75-150 µs per lookup ≈ ~1.5% of one core.
//! WAL mode + indexed PK lookup; readers don't block writers.

use std::sync::Arc;

use async_trait::async_trait;
use rekindle_transport::envelope_store::StoreError;
use rekindle_transport::friend_store::{FriendRecord, FriendStatus, FriendStore};

use crate::db::DbPool;
use crate::db_helpers::db_call;

/// SQLite-backed [`FriendStore`]. Cheap to clone (single `Arc<DbPool>` inside).
pub struct SqliteFriendStore {
    pool: Arc<DbPool>,
}

impl SqliteFriendStore {
    pub fn new(pool: Arc<DbPool>) -> Self {
        Self { pool }
    }

    /// Convenience: wrap as the trait object expected by transport.
    pub fn into_dyn(self) -> Arc<dyn FriendStore> {
        Arc::new(self)
    }
}

fn map_db_err(reason: String) -> StoreError {
    StoreError::Other(reason)
}

/// Project a single `friends` row to a [`FriendRecord`]. The column order
/// is fixed by [`row_to_record`]'s `prepare()` call sites — keep them in sync.
fn row_to_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<FriendRecord> {
    Ok(FriendRecord {
        pubkey_hex: row.get::<_, String>(0)?,
        inbox_record_key: row.get::<_, Option<String>>(1)?.unwrap_or_default(),
        mailbox_record_key: row.get::<_, Option<String>>(2)?.unwrap_or_default(),
        current_device_id: row.get::<_, Option<String>>(3)?,
        display_name: row.get::<_, Option<String>>(4)?.unwrap_or_default(),
        added_at_us: ms_to_us(row.get::<_, i64>(5)?),
        status: FriendStatus::from_wire(&row.get::<_, String>(6)?),
    })
}

/// `friends.added_at` is stored as **milliseconds** since epoch (existing
/// schema); the transport-side `FriendRecord::added_at_us` is microseconds.
/// Convert at the boundary.
fn ms_to_us(ms: i64) -> u64 {
    u64::try_from(ms).unwrap_or(0).saturating_mul(1_000)
}

const SELECT_COLS: &str = "public_key, dht_record_key, mailbox_dht_key, \
    current_device_id, display_name, added_at, friendship_state";

#[async_trait]
impl FriendStore for SqliteFriendStore {
    async fn lookup_by_pubkey(
        &self,
        owner_key: &str,
        pubkey_hex: &str,
    ) -> Result<Option<FriendRecord>, StoreError> {
        let owner = owner_key.to_string();
        let pubkey = pubkey_hex.to_string();

        db_call(&self.pool, move |conn| {
            let sql = format!(
                "SELECT {SELECT_COLS} FROM friends \
                 WHERE owner_key = ?1 AND public_key = ?2"
            );
            let mut stmt = conn.prepare(&sql)?;
            let mut rows = stmt.query(rusqlite::params![owner, pubkey])?;
            if let Some(row) = rows.next()? {
                Ok(Some(row_to_record(row)?))
            } else {
                Ok(None)
            }
        })
        .await
        .map_err(map_db_err)
    }

    async fn lookup_by_inbox_record_key(
        &self,
        owner_key: &str,
        inbox_record_key: &str,
    ) -> Result<Option<FriendRecord>, StoreError> {
        let owner = owner_key.to_string();
        let key = inbox_record_key.to_string();

        db_call(&self.pool, move |conn| {
            let sql = format!(
                "SELECT {SELECT_COLS} FROM friends \
                 WHERE owner_key = ?1 AND dht_record_key = ?2"
            );
            let mut stmt = conn.prepare(&sql)?;
            let mut rows = stmt.query(rusqlite::params![owner, key])?;
            if let Some(row) = rows.next()? {
                Ok(Some(row_to_record(row)?))
            } else {
                Ok(None)
            }
        })
        .await
        .map_err(map_db_err)
    }

    async fn lookup_batch_by_pubkey(
        &self,
        owner_key: &str,
        pubkey_hexes: &[String],
    ) -> Result<Vec<FriendRecord>, StoreError> {
        if pubkey_hexes.is_empty() {
            return Ok(Vec::new());
        }
        let owner = owner_key.to_string();
        let pubkeys: Vec<String> = pubkey_hexes.to_vec();

        db_call(&self.pool, move |conn| {
            // Build `?2, ?3, ..., ?N` placeholders. Using rarray_v2 is
            // overkill for the scale we need (typically ≤8 subkeys per
            // VeilidValueChange); plain placeholders keep the dependency
            // surface minimal.
            let placeholders: String = (2..=pubkeys.len() + 1)
                .map(|i| format!("?{i}"))
                .collect::<Vec<_>>()
                .join(", ");
            let sql = format!(
                "SELECT {SELECT_COLS} FROM friends \
                 WHERE owner_key = ?1 AND public_key IN ({placeholders})"
            );
            let mut stmt = conn.prepare(&sql)?;

            let mut params: Vec<&dyn rusqlite::ToSql> = Vec::with_capacity(1 + pubkeys.len());
            params.push(&owner);
            for pubkey in &pubkeys {
                params.push(pubkey);
            }

            let rows = stmt.query_map(params.as_slice(), row_to_record)?;
            let mut out = Vec::with_capacity(pubkeys.len());
            for row in rows {
                out.push(row?);
            }
            Ok(out)
        })
        .await
        .map_err(map_db_err)
    }

    async fn iter_active(&self, owner_key: &str) -> Result<Vec<FriendRecord>, StoreError> {
        let owner = owner_key.to_string();

        db_call(&self.pool, move |conn| {
            let sql = format!(
                "SELECT {SELECT_COLS} FROM friends \
                 WHERE owner_key = ?1 AND friendship_state = 'accepted'"
            );
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(rusqlite::params![owner], row_to_record)?;
            let mut out = Vec::new();
            for row in rows {
                out.push(row?);
            }
            Ok(out)
        })
        .await
        .map_err(map_db_err)
    }

    // is_active_friend uses the trait default (lookup_by_pubkey + status check).
    // The default does one SQLite round-trip, which is fine for the call rate;
    // a custom `SELECT 1 FROM friends WHERE ... AND friendship_state = 'accepted'`
    // would shave µs but adds a divergent code path. Keep the default.
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio_rusqlite::Connection as TokioConn;

    async fn fresh_db_with_friend(owner: &str, pubkey: &str, state: &str) -> Arc<DbPool> {
        let pool = TokioConn::open_in_memory().await.unwrap();
        let owner_owned = owner.to_string();
        let pubkey_owned = pubkey.to_string();
        let state_owned = state.to_string();
        pool.call(move |conn| -> rusqlite::Result<()> {
            // Minimal schema for the test — only the columns SqliteFriendStore reads.
            conn.execute_batch(
                "CREATE TABLE identity (public_key TEXT PRIMARY KEY);
                 CREATE TABLE friends (
                    owner_key TEXT NOT NULL,
                    public_key TEXT NOT NULL,
                    display_name TEXT,
                    nickname TEXT,
                    group_id INTEGER,
                    added_at INTEGER NOT NULL,
                    dht_record_key TEXT,
                    last_seen_at INTEGER,
                    avatar_webp BLOB,
                    local_conversation_key TEXT,
                    local_conversation_keypair TEXT,
                    remote_conversation_key TEXT,
                    mailbox_dht_key TEXT,
                    friendship_state TEXT NOT NULL DEFAULT 'accepted',
                    current_device_id TEXT,
                    PRIMARY KEY (owner_key, public_key)
                 );",
            )?;
            conn.execute(
                "INSERT INTO identity (public_key) VALUES (?1)",
                rusqlite::params![&owner_owned],
            )?;
            conn.execute(
                "INSERT INTO friends (
                    owner_key, public_key, display_name, added_at,
                    dht_record_key, mailbox_dht_key, friendship_state,
                    current_device_id
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                rusqlite::params![
                    &owner_owned,
                    &pubkey_owned,
                    "Alice",
                    1_700_000_000_000_i64, // ms
                    "inbox-key-hex",
                    "mailbox-key-hex",
                    &state_owned,
                    "device-id-hex",
                ],
            )?;
            Ok(())
        })
        .await
        .unwrap();
        Arc::new(pool)
    }

    #[tokio::test]
    async fn lookup_by_pubkey_round_trip() {
        let pool = fresh_db_with_friend("me", "alice", "accepted").await;
        let store = SqliteFriendStore::new(pool);

        let got = store.lookup_by_pubkey("me", "alice").await.unwrap();
        let record = got.expect("alice should be present");
        assert_eq!(record.pubkey_hex, "alice");
        assert_eq!(record.inbox_record_key, "inbox-key-hex");
        assert_eq!(record.mailbox_record_key, "mailbox-key-hex");
        assert_eq!(record.current_device_id.as_deref(), Some("device-id-hex"));
        assert_eq!(record.display_name, "Alice");
        assert_eq!(record.added_at_us, 1_700_000_000_000_000);
        assert_eq!(record.status, FriendStatus::Active);
    }

    #[tokio::test]
    async fn lookup_by_pubkey_misses_other_owner() {
        let pool = fresh_db_with_friend("me", "alice", "accepted").await;
        let store = SqliteFriendStore::new(pool);

        let got = store.lookup_by_pubkey("you", "alice").await.unwrap();
        assert!(got.is_none());
    }

    #[tokio::test]
    async fn is_active_friend_distinguishes_states() {
        let pool_active = fresh_db_with_friend("me", "alice", "accepted").await;
        let store_active = SqliteFriendStore::new(pool_active);
        assert!(store_active.is_active_friend("me", "alice").await.unwrap());

        let pool_pending = fresh_db_with_friend("me", "bob", "pending_out").await;
        let store_pending = SqliteFriendStore::new(pool_pending);
        assert!(!store_pending.is_active_friend("me", "bob").await.unwrap());

        let pool_removing = fresh_db_with_friend("me", "carol", "removing").await;
        let store_removing = SqliteFriendStore::new(pool_removing);
        assert!(!store_removing
            .is_active_friend("me", "carol")
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn lookup_by_inbox_record_key_finds_match() {
        let pool = fresh_db_with_friend("me", "alice", "accepted").await;
        let store = SqliteFriendStore::new(pool);

        let got = store
            .lookup_by_inbox_record_key("me", "inbox-key-hex")
            .await
            .unwrap();
        assert_eq!(got.unwrap().pubkey_hex, "alice");

        let miss = store
            .lookup_by_inbox_record_key("me", "nonexistent")
            .await
            .unwrap();
        assert!(miss.is_none());
    }

    #[tokio::test]
    async fn lookup_batch_returns_only_matches() {
        let pool = fresh_db_with_friend("me", "alice", "accepted").await;
        let store = SqliteFriendStore::new(pool);

        let want: Vec<String> = vec!["alice".into(), "ghost".into()];
        let got = store.lookup_batch_by_pubkey("me", &want).await.unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].pubkey_hex, "alice");
    }

    #[tokio::test]
    async fn lookup_batch_empty_input_returns_empty() {
        let pool = fresh_db_with_friend("me", "alice", "accepted").await;
        let store = SqliteFriendStore::new(pool);
        let got = store.lookup_batch_by_pubkey("me", &[]).await.unwrap();
        assert!(got.is_empty());
    }

    #[tokio::test]
    async fn iter_active_filters_pending_and_removing() {
        let pool = fresh_db_with_friend("me", "alice", "accepted").await;
        // Add a pending and a removing friend to the same owner.
        pool.call(|conn| -> rusqlite::Result<()> {
            conn.execute(
                "INSERT INTO friends (
                    owner_key, public_key, display_name, added_at,
                    dht_record_key, mailbox_dht_key, friendship_state, current_device_id
                 ) VALUES ('me', 'bob', 'Bob', 1, 'i', 'm', 'pending_out', NULL),
                          ('me', 'carol', 'Carol', 1, 'i2', 'm2', 'removing', NULL)",
                [],
            )?;
            Ok(())
        })
        .await
        .unwrap();

        let store = SqliteFriendStore::new(pool);
        let active = store.iter_active("me").await.unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].pubkey_hex, "alice");
    }
}
