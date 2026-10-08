//! SQLite-backed [`FriendStore`]: the receive path's friend authority,
//! read from the `friends` table every host shares
//! ([`crate::repo::friends`]).
//!
//! # Hot path
//!
//! [`FriendStore::lookup_by_pubkey`] is called once per inbound envelope.
//! At realistic peak throughput (~100 envelopes/sec across active chat,
//! presence, and voice setup), ~75-150 µs per lookup ≈ ~1.5% of one core.
//! WAL mode + indexed PK lookup; readers don't block writers.

use std::sync::Arc;

use async_trait::async_trait;
use rekindle_types::friend_store::{FriendRecord, FriendStore, FriendStoreError};

use crate::repo::friends;
use crate::Db;

/// SQLite-backed [`FriendStore`].
pub struct SqliteFriendStore {
    db: Db,
}

impl SqliteFriendStore {
    #[must_use]
    pub fn new(db: Db) -> Self {
        Self { db }
    }

    /// Convenience: wrap as the trait object expected by transport.
    pub fn into_dyn(self) -> Arc<dyn FriendStore> {
        Arc::new(self)
    }
}

fn map_db_err(e: impl std::fmt::Display) -> FriendStoreError {
    FriendStoreError::Backend(e.to_string())
}

#[async_trait]
impl FriendStore for SqliteFriendStore {
    async fn lookup_by_pubkey(
        &self,
        owner_key: &str,
        pubkey_hex: &str,
    ) -> Result<Option<FriendRecord>, FriendStoreError> {
        let owner = owner_key.to_string();
        let pubkey = pubkey_hex.to_string();
        self.db
            .call(move |conn| friends::record(conn, &owner, &pubkey))
            .await
            .map_err(map_db_err)
    }

    async fn lookup_by_inbox_record_key(
        &self,
        owner_key: &str,
        inbox_record_key: &str,
    ) -> Result<Option<FriendRecord>, FriendStoreError> {
        let owner = owner_key.to_string();
        let key = inbox_record_key.to_string();
        self.db
            .call(move |conn| friends::record_by_profile_key(conn, &owner, &key))
            .await
            .map_err(map_db_err)
    }

    async fn lookup_batch_by_pubkey(
        &self,
        owner_key: &str,
        pubkey_hexes: &[String],
    ) -> Result<Vec<FriendRecord>, FriendStoreError> {
        if pubkey_hexes.is_empty() {
            return Ok(Vec::new());
        }
        let owner = owner_key.to_string();
        let pubkeys = pubkey_hexes.to_vec();
        self.db
            .call(move |conn| friends::records(conn, &owner, &pubkeys))
            .await
            .map_err(map_db_err)
    }

    async fn iter_active(&self, owner_key: &str) -> Result<Vec<FriendRecord>, FriendStoreError> {
        let owner = owner_key.to_string();
        self.db
            .call(move |conn| friends::active_records(conn, &owner))
            .await
            .map_err(map_db_err)
    }

    // is_active_friend uses the trait default (lookup_by_pubkey + status
    // check): one round trip, and no second query to keep in step.
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::fixture::{self, OWNER};
    use rekindle_types::friend_store::FriendStatus;

    /// A store over the real schema with one friend per `(key, state)`.
    fn store_with(friends: &[(&str, &str)]) -> SqliteFriendStore {
        let conn = fixture::conn();
        for (i, (key, state)) in friends.iter().enumerate() {
            conn.execute(
                "INSERT INTO friends (owner_key, public_key, display_name, added_at, \
                 dht_record_key, mailbox_dht_key, friendship_state, current_device_id) \
                 VALUES (?1, ?2, 'Alice', 1700000000000, ?3, ?4, ?5, 'device-id-hex')",
                rusqlite::params![
                    OWNER,
                    key,
                    format!("inbox-{i}"),
                    format!("mailbox-{i}"),
                    state
                ],
            )
            .unwrap();
        }
        SqliteFriendStore::new(Db::from(rekindle_asql::Connection::from(conn)))
    }

    #[tokio::test]
    async fn lookup_by_pubkey_round_trip() {
        let store = store_with(&[("alice", "accepted")]);
        let record = store
            .lookup_by_pubkey(OWNER, "alice")
            .await
            .unwrap()
            .expect("alice should be present");
        assert_eq!(record.pubkey_hex, "alice");
        assert_eq!(record.inbox_record_key, "inbox-0");
        assert_eq!(record.mailbox_record_key, "mailbox-0");
        assert_eq!(record.current_device_id.as_deref(), Some("device-id-hex"));
        assert_eq!(record.display_name, "Alice");
        assert_eq!(record.added_at_us, 1_700_000_000_000_000);
        assert_eq!(record.status, FriendStatus::Active);
    }

    #[tokio::test]
    async fn lookup_by_pubkey_misses_other_owner() {
        let store = store_with(&[("alice", "accepted")]);
        assert!(store
            .lookup_by_pubkey("you", "alice")
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn is_active_friend_distinguishes_states() {
        let store = store_with(&[
            ("alice", "accepted"),
            ("bob", "pending_out"),
            ("carol", "removing"),
        ]);
        assert!(store.is_active_friend(OWNER, "alice").await.unwrap());
        assert!(!store.is_active_friend(OWNER, "bob").await.unwrap());
        assert!(!store.is_active_friend(OWNER, "carol").await.unwrap());
    }

    #[tokio::test]
    async fn lookup_by_inbox_record_key_finds_match() {
        let store = store_with(&[("alice", "accepted")]);
        let got = store
            .lookup_by_inbox_record_key(OWNER, "inbox-0")
            .await
            .unwrap();
        assert_eq!(got.unwrap().pubkey_hex, "alice");
        assert!(store
            .lookup_by_inbox_record_key(OWNER, "nonexistent")
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn lookup_batch_returns_only_matches() {
        let store = store_with(&[("alice", "accepted")]);
        let want: Vec<String> = vec!["alice".into(), "ghost".into()];
        let got = store.lookup_batch_by_pubkey(OWNER, &want).await.unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].pubkey_hex, "alice");
        assert!(store
            .lookup_batch_by_pubkey(OWNER, &[])
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn iter_active_filters_pending_and_removing() {
        let store = store_with(&[
            ("alice", "accepted"),
            ("bob", "pending_out"),
            ("carol", "removing"),
        ]);
        let active = store.iter_active(OWNER).await.unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].pubkey_hex, "alice");
    }
}
