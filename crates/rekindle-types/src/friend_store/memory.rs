//! In-memory [`FriendStore`] for tests. No I/O. No persistence — every
//! record is dropped when the struct is dropped. Used in unit tests across
//! the workspace and as a stand-in when the host hasn't wired a real
//! store yet.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::RwLock;

use super::FriendStoreError;
use super::{FriendRecord, FriendStore, FriendshipState};

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
    pub fn upsert(&self, owner_key: impl Into<String>, record: FriendRecord) {
        let mut map = self.inner.write();
        let key = (owner_key.into(), record.pubkey_hex.clone());
        map.insert(key, record);
    }

    /// Test helper — remove a friend record.
    pub fn remove(&self, owner_key: &str, pubkey_hex: &str) -> Option<FriendRecord> {
        let mut map = self.inner.write();
        map.remove(&(owner_key.to_string(), pubkey_hex.to_string()))
    }
}

#[async_trait]
impl FriendStore for MemoryFriendStore {
    async fn lookup_by_pubkey(
        &self,
        owner_key: &str,
        pubkey_hex: &str,
    ) -> Result<Option<FriendRecord>, FriendStoreError> {
        let map = self.inner.read();
        Ok(map
            .get(&(owner_key.to_string(), pubkey_hex.to_string()))
            .cloned())
    }

    async fn lookup_by_inbox_record_key(
        &self,
        owner_key: &str,
        inbox_record_key: &str,
    ) -> Result<Option<FriendRecord>, FriendStoreError> {
        let map = self.inner.read();
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
    ) -> Result<Vec<FriendRecord>, FriendStoreError> {
        let map = self.inner.read();
        Ok(pubkey_hexes
            .iter()
            .filter_map(|pubkey| map.get(&(owner_key.to_string(), pubkey.clone())).cloned())
            .collect())
    }

    async fn iter_active(&self, owner_key: &str) -> Result<Vec<FriendRecord>, FriendStoreError> {
        let map = self.inner.read();
        Ok(map
            .iter()
            .filter(|((owner, _), record)| {
                owner == owner_key && record.status == FriendshipState::Accepted
            })
            .map(|(_, record)| record.clone())
            .collect())
    }
}

#[cfg(test)]
#[path = "memory_tests.rs"]
mod tests;
