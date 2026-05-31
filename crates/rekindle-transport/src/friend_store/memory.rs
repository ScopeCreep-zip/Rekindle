//! In-memory [`FriendStore`] for tests. No I/O. No persistence — every
//! record is dropped when the struct is dropped. Used in unit tests across
//! the workspace and as a stand-in when the host hasn't wired a real
//! store yet.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::RwLock;

use super::{FriendRecord, FriendStatus, FriendStore};
use crate::envelope_store::StoreError;

/// Composite key: `(owner_key, friend_pubkey_hex)`. Matches the SQLite
/// `friends` table primary key.
type Key = (String, String);

/// In-memory friend store. Cheap to construct; cloneable via `Arc`.
#[derive(Debug, Default, Clone)]
pub struct MemoryFriendStore {
    inner: Arc<RwLock<HashMap<Key, FriendRecord>>>,
}

impl MemoryFriendStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Test helper — insert or replace a friend record.
    pub async fn upsert(&self, owner_key: impl Into<String>, record: FriendRecord) {
        let mut map = self.inner.write().await;
        let key = (owner_key.into(), record.pubkey_hex.clone());
        map.insert(key, record);
    }

    /// Test helper — remove a friend record.
    pub async fn remove(&self, owner_key: &str, pubkey_hex: &str) -> Option<FriendRecord> {
        let mut map = self.inner.write().await;
        map.remove(&(owner_key.to_string(), pubkey_hex.to_string()))
    }
}

#[async_trait]
impl FriendStore for MemoryFriendStore {
    async fn lookup_by_pubkey(
        &self,
        owner_key: &str,
        pubkey_hex: &str,
    ) -> Result<Option<FriendRecord>, StoreError> {
        let map = self.inner.read().await;
        Ok(map
            .get(&(owner_key.to_string(), pubkey_hex.to_string()))
            .cloned())
    }

    async fn lookup_by_inbox_record_key(
        &self,
        owner_key: &str,
        inbox_record_key: &str,
    ) -> Result<Option<FriendRecord>, StoreError> {
        let map = self.inner.read().await;
        Ok(map
            .iter()
            .find(|((owner, _), record)| {
                owner == owner_key && record.inbox_record_key == inbox_record_key
            })
            .map(|(_, record)| record.clone()))
    }

    async fn lookup_batch_by_pubkey(
        &self,
        owner_key: &str,
        pubkey_hexes: &[String],
    ) -> Result<Vec<FriendRecord>, StoreError> {
        let map = self.inner.read().await;
        Ok(pubkey_hexes
            .iter()
            .filter_map(|pubkey| map.get(&(owner_key.to_string(), pubkey.clone())).cloned())
            .collect())
    }

    async fn iter_active(&self, owner_key: &str) -> Result<Vec<FriendRecord>, StoreError> {
        let map = self.inner.read().await;
        Ok(map
            .iter()
            .filter(|((owner, _), record)| {
                owner == owner_key && record.status == FriendStatus::Active
            })
            .map(|(_, record)| record.clone())
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(pubkey: &str, status: FriendStatus) -> FriendRecord {
        FriendRecord {
            pubkey_hex: pubkey.to_string(),
            inbox_record_key: format!("inbox-{pubkey}"),
            mailbox_record_key: format!("mailbox-{pubkey}"),
            current_device_id: None,
            display_name: format!("friend-{pubkey}"),
            added_at_us: 1_700_000_000_000_000,
            status,
        }
    }

    #[tokio::test]
    async fn lookup_by_pubkey_hits_active() {
        let store = MemoryFriendStore::new();
        store
            .upsert("me", record("alice", FriendStatus::Active))
            .await;

        let got = store.lookup_by_pubkey("me", "alice").await.unwrap();
        assert_eq!(got.unwrap().display_name, "friend-alice");
    }

    #[tokio::test]
    async fn lookup_by_pubkey_misses_other_owner() {
        let store = MemoryFriendStore::new();
        store
            .upsert("me", record("alice", FriendStatus::Active))
            .await;

        // Different owner_key — should not see "me"'s friend.
        let got = store.lookup_by_pubkey("you", "alice").await.unwrap();
        assert!(got.is_none());
    }

    #[tokio::test]
    async fn is_active_friend_excludes_pending_and_removing() {
        let store = MemoryFriendStore::new();
        store
            .upsert("me", record("alice", FriendStatus::Active))
            .await;
        store
            .upsert("me", record("bob", FriendStatus::PendingOut))
            .await;
        store
            .upsert("me", record("carol", FriendStatus::Removing))
            .await;

        assert!(store.is_active_friend("me", "alice").await.unwrap());
        assert!(!store.is_active_friend("me", "bob").await.unwrap());
        assert!(!store.is_active_friend("me", "carol").await.unwrap());
        assert!(!store.is_active_friend("me", "dave").await.unwrap()); // unknown
    }

    #[tokio::test]
    async fn iter_active_filters_status_and_owner() {
        let store = MemoryFriendStore::new();
        store
            .upsert("me", record("alice", FriendStatus::Active))
            .await;
        store
            .upsert("me", record("bob", FriendStatus::PendingOut))
            .await;
        store
            .upsert("you", record("eve", FriendStatus::Active))
            .await;

        let active = store.iter_active("me").await.unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].pubkey_hex, "alice");
    }

    #[tokio::test]
    async fn lookup_by_inbox_record_key_finds_match() {
        let store = MemoryFriendStore::new();
        store
            .upsert("me", record("alice", FriendStatus::Active))
            .await;

        let got = store
            .lookup_by_inbox_record_key("me", "inbox-alice")
            .await
            .unwrap();
        assert_eq!(got.unwrap().pubkey_hex, "alice");

        let miss = store
            .lookup_by_inbox_record_key("me", "inbox-nobody")
            .await
            .unwrap();
        assert!(miss.is_none());
    }

    #[tokio::test]
    async fn lookup_batch_by_pubkey_returns_only_known() {
        let store = MemoryFriendStore::new();
        store
            .upsert("me", record("alice", FriendStatus::Active))
            .await;
        store
            .upsert("me", record("bob", FriendStatus::PendingOut))
            .await;

        let want: Vec<String> = vec!["alice".into(), "bob".into(), "carol".into()];
        let got = store.lookup_batch_by_pubkey("me", &want).await.unwrap();
        assert_eq!(got.len(), 2);
        let pubkeys: Vec<&str> = got.iter().map(|r| r.pubkey_hex.as_str()).collect();
        assert!(pubkeys.contains(&"alice"));
        assert!(pubkeys.contains(&"bob"));
    }

    #[tokio::test]
    async fn from_wire_unknown_falls_back_to_removing() {
        assert_eq!(FriendStatus::from_wire("garbage"), FriendStatus::Removing);
        assert_eq!(FriendStatus::from_wire("accepted"), FriendStatus::Active);
        assert_eq!(
            FriendStatus::from_wire("pending_out"),
            FriendStatus::PendingOut
        );
        assert_eq!(FriendStatus::from_wire("removing"), FriendStatus::Removing);
    }
}
