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
//!
//! # No database of its own
//!
//! The store resolves [`DbHandle::current`] on every call and holds no
//! [`Db`]: a clone held for the app's life kept the identity database open
//! past `DbHandle::clear` at exit and logout (plan C7.18). With no identity
//! loaded a lookup is an error, which the receive path fails closed on.

use std::sync::Arc;

use async_trait::async_trait;
use rekindle_types::friend_store::{FriendRecord, FriendStore, FriendStoreError};

use crate::repo::friends;
use crate::{Db, DbHandle};

/// SQLite-backed [`FriendStore`].
pub struct SqliteFriendStore {
    handle: Arc<DbHandle>,
}

impl SqliteFriendStore {
    #[must_use]
    pub fn new(handle: Arc<DbHandle>) -> Self {
        Self { handle }
    }

    /// The open identity database, or a backend error when none is.
    fn db(&self) -> Result<Db, FriendStoreError> {
        self.handle.current().map_err(map_db_err)
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
        self.db()?
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
        self.db()?
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
        self.db()?
            .call(move |conn| friends::records(conn, &owner, &pubkeys))
            .await
            .map_err(map_db_err)
    }

    async fn iter_active(&self, owner_key: &str) -> Result<Vec<FriendRecord>, FriendStoreError> {
        let owner = owner_key.to_string();
        self.db()?
            .call(move |conn| friends::active_records(conn, &owner))
            .await
            .map_err(map_db_err)
    }

    // is_active_friend uses the trait default (lookup_by_pubkey + status
    // check): one round trip, and no second query to keep in step.
}

#[cfg(test)]
#[path = "friend_store_tests.rs"]
mod tests;
