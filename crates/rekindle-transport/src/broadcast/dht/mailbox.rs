//! Mailbox DHT record operations (DFLT, 1 subkey).
//!
//! The mailbox record stores only the node's current route blob. It uses
//! a deterministic key derived from the identity keypair, making it
//! permanent and shareable in invite links.

use veilid_core::{KeyPair, RoutingContext};

use super::record;
use crate::error::Result;

/// Operations on a mailbox DHT record.
pub struct MailboxOps<'a> {
    rc: &'a RoutingContext,
}

impl<'a> MailboxOps<'a> {
    pub(crate) fn new(rc: &'a RoutingContext) -> Self {
        Self { rc }
    }

    /// Create a mailbox record with a deterministic key from the identity keypair.
    ///
    /// Returns the record key string.
    pub async fn create(&self, identity_keypair: KeyPair) -> Result<String> {
        let (key, _) = record::create_dflt(self.rc, 1, Some(identity_keypair)).await?;
        tracing::info!(key = %key, "mailbox record created");
        Ok(key)
    }

    /// Open our own mailbox for writing.
    pub async fn open_writable(&self, key: &str, identity_keypair: KeyPair) -> Result<()> {
        record::open_writable(self.rc, key, identity_keypair).await
    }

    /// Update our route blob in the mailbox.
    pub async fn update_route(&self, key: &str, route_blob: &[u8]) -> Result<()> {
        record::set(self.rc, key, 0, route_blob.to_vec(), None).await
    }

    /// Read a peer's route blob from their mailbox.
    pub async fn read_peer_route(&self, mailbox_key: &str) -> Result<Option<Vec<u8>>> {
        record::open_readonly(self.rc, mailbox_key).await?;
        record::get(self.rc, mailbox_key, 0, true).await
    }

    /// Close the mailbox record.
    pub async fn close(&self, key: &str) -> Result<()> {
        record::close(self.rc, key).await
    }

    // ── Community mailbox ───────────────────────────────────────────

    /// Create a community mailbox owned by the governance keypair.
    pub async fn create_community_mailbox(
        &self,
        governance_keypair: KeyPair,
    ) -> Result<String> {
        let (key, _) = record::create_dflt(self.rc, 1, Some(governance_keypair)).await?;
        tracing::info!(key = %key, "community mailbox created");
        Ok(key)
    }

    /// Update the community mailbox with a fresh route blob.
    pub async fn update_community_route(
        &self,
        mailbox_key: &str,
        route_blob: &[u8],
    ) -> Result<()> {
        record::set(self.rc, mailbox_key, 0, route_blob.to_vec(), None).await
    }

    /// Read the community route blob (for joiners sending RPC).
    pub async fn read_community_route(
        &self,
        mailbox_key: &str,
    ) -> Result<Option<Vec<u8>>> {
        record::open_readonly(self.rc, mailbox_key).await?;
        record::get(self.rc, mailbox_key, 0, true).await
    }
}
