//! Friend list DHT record operations (DFLT, 1 subkey).
//!
//! The entire friend list is stored as a single postcard-serialized blob
//! in subkey 0 of a DFLT(1) record.

use veilid_core::{KeyPair, RoutingContext};

use super::record;
use crate::error::Result;
use crate::payload::dht_types::{FriendEntry, FriendList};

/// Operations on a friend list DHT record.
pub struct FriendListOps<'a> {
    rc: &'a RoutingContext,
}

impl<'a> FriendListOps<'a> {
    pub(crate) fn new(rc: &'a RoutingContext) -> Self {
        Self { rc }
    }

    /// Create a new empty friend list record.
    ///
    /// Returns `(key, keypair)`. The keypair MUST be persisted.
    pub async fn create(&self) -> Result<(String, Option<KeyPair>)> {
        let (key, keypair) = record::create_dflt(self.rc, 1, None).await?;

        let empty = FriendList::default();
        let data = postcard::to_stdvec(&empty)
            .map_err(|e| crate::error::TransportError::SerializationFailed { reason: e.to_string() })?;
        record::set(self.rc, &key, 0, data, None).await?;

        tracing::info!(key = %key, "friend list record created");
        Ok((key, keypair))
    }

    /// Open or create a friend list record.
    ///
    /// Returns `(key, keypair, is_new)`.
    pub async fn open_or_create(
        &self,
        existing_key: Option<&str>,
        existing_keypair: Option<KeyPair>,
    ) -> Result<(String, Option<KeyPair>, bool)> {
        let (key, keypair, is_new) = record::open_or_create(
            self.rc, existing_key, existing_keypair, 1,
        ).await?;

        if is_new {
            let empty = FriendList::default();
            let data = postcard::to_stdvec(&empty)
                .map_err(|e| crate::error::TransportError::SerializationFailed { reason: e.to_string() })?;
            record::set(self.rc, &key, 0, data, None).await?;
            tracing::info!(key = %key, "friend list record created");
        }

        Ok((key, keypair, is_new))
    }

    /// Read the full friend list.
    pub async fn read(&self, key: &str) -> Result<FriendList> {
        match record::get(self.rc, key, 0, false).await? {
            Some(data) => postcard::from_bytes(&data)
                .map_err(|e| crate::error::TransportError::DeserializationFailed {
                    type_id: 0,
                    reason: format!("friend list: {e}"),
                }),
            None => Ok(FriendList::default()),
        }
    }

    /// Add a friend to the list (deduplicates by public_key).
    pub async fn add(&self, key: &str, entry: FriendEntry) -> Result<()> {
        let mut list = self.read(key).await?;
        if list.friends.iter().any(|f| f.public_key == entry.public_key) {
            return Ok(());
        }
        list.friends.push(entry);
        self.write(key, &list).await
    }

    /// Remove a friend by public key.
    pub async fn remove(&self, key: &str, public_key: &str) -> Result<()> {
        let mut list = self.read(key).await?;
        list.friends.retain(|f| f.public_key != public_key);
        self.write(key, &list).await
    }

    /// Close the record.
    pub async fn close(&self, key: &str) -> Result<()> {
        record::close(self.rc, key).await
    }

    async fn write(&self, key: &str, list: &FriendList) -> Result<()> {
        let data = postcard::to_stdvec(list)
            .map_err(|e| crate::error::TransportError::SerializationFailed { reason: e.to_string() })?;
        record::set(self.rc, key, 0, data, None).await
    }
}
