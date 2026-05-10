//! Friendship lifecycle — request, accept, reject, remove, inbox scan.
//!
//! Orchestrates PQXDH handshake (via `rekindle-ratchet`), DhtLog creation
//! (via `PlatformIO`), session persistence (via `rekindle-storage`),
//! and inbox discovery (3-tier: watch + poll + direct notification).

pub mod request;
pub mod accept;
pub mod reject;
pub mod remove;
pub mod inbox;
pub mod respond;

use std::sync::Arc;

use parking_lot::RwLock;
use rekindle_storage::VaultStore;
use rekindle_types::session_types::SessionMeta;

use crate::crypto::sessions::SessionCache;
use crate::events::registry::WatchRegistry;
use crate::io::PlatformIO;
use crate::ChatError;

/// Friendship service — all I/O through PlatformIO, no direct transport access.
pub struct FriendshipService {
    pub(crate) io: Arc<PlatformIO>,
    pub(crate) vault: Arc<VaultStore>,
    pub(crate) session_meta: Arc<RwLock<SessionMeta>>,
    pub(crate) session_cache: Arc<SessionCache>,
    pub(crate) watches: Arc<WatchRegistry>,
    pub(crate) inbox_trigger: tokio::sync::mpsc::Sender<()>,
}

impl FriendshipService {
    /// List pending friend requests from session metadata.
    pub fn list_pending(&self) -> Vec<rekindle_types::session_types::PendingFriendRequest> {
        let meta = self.session_meta.read();
        meta.pending_friend_requests.clone()
    }

    /// Trigger an inbox scan (non-blocking). Called by event router
    /// when a FriendRequestAck arrives or a friend inbox watch fires.
    /// Sends to the InboxScanCoordinator's mpsc channel. Dropped if
    /// the channel is full (coalesced by the coordinator's 30s cooldown).
    pub fn trigger_inbox_scan(&self) {
        let _ = self.inbox_trigger.try_send(());
    }

    /// Get the identity from session meta, or error if not initialized.
    pub(crate) fn require_identity(
        &self,
    ) -> Result<rekindle_types::session_types::SessionIdentity, ChatError> {
        let meta = self.session_meta.read();
        meta.identity
            .clone()
            .ok_or(ChatError::NotInitialized)
    }

    /// Handle an inbound unfriend notification from a peer.
    pub async fn handle_unfriend(&self, sender_key: &str) {
        tracing::info!(
            peer = &sender_key[..12.min(sender_key.len())],
            "unfriend notification received — removing friend"
        );
        if let Err(e) = self.remove_friend(sender_key).await {
            tracing::error!(
                peer = &sender_key[..12.min(sender_key.len())],
                error = %e,
                "unfriend handling FAILED — friend may remain in local state"
            );
        }
    }

    /// Handle a profile key rotation notification from a peer.
    ///
    /// Updates the peer's profile_dht_key in the friend list DHT record
    /// and invalidates the cached route (the peer's route blob is published
    /// to their profile record, which has moved to the new key).
    pub async fn handle_profile_rotated(&self, sender_key: &str, payload: &[u8]) {
        let dm_payload: rekindle_types::dm_payload::DmPayload = match postcard::from_bytes(payload) {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(
                    peer = &sender_key[..12.min(sender_key.len())],
                    error = %e,
                    "profile rotation parse failed"
                );
                return;
            }
        };
        if let rekindle_types::dm_payload::DmPayload::ProfileKeyRotated { new_profile_dht_key } = dm_payload {
            tracing::info!(
                peer = &sender_key[..12.min(sender_key.len())],
                new_key = &new_profile_dht_key[..12.min(new_profile_dht_key.len())],
                "peer rotated profile DHT key"
            );

            // Invalidate cached route — the old profile record's route blob
            // is stale, the peer is now reachable via the new profile record.
            self.io.invalidate_peer_route(sender_key);

            // Update the peer's profile_dht_key in the friend list DHT record
            // so future interactions (prekey fetch, route lookup) use the new key.
            let fl_keypair = match self.vault.load_key(
                rekindle_storage::keys::labels::FRIEND_LIST_KEYPAIR,
            ) {
                Ok(kp) => kp,
                Err(e) => {
                    tracing::warn!(error = %e, "cannot load friend list keypair for profile rotation update");
                    return;
                }
            };
            let Some(ref kp) = fl_keypair else { return };

            let fl_key = {
                let meta = self.session_meta.read();
                meta.identity.as_ref().map(|i| i.friend_list_dht_key.clone()).unwrap_or_default()
            };
            if fl_key.is_empty() { return; }

            let Ok(Some(existing)) = self.io.read_record(&fl_key, 0, false).await else { return };

            let mut friend_list: rekindle_types::dht_types::FriendList =
                if existing.is_empty() || existing == b"[]" {
                    return; // no friends to update
                } else {
                    serde_json::from_slice(&existing).unwrap_or_default()
                };

            // Find the friend entry for this peer and update their profile key
            let Some(entry) = friend_list.friends.iter_mut().find(|f| f.public_key == sender_key) else {
                return; // peer not in friend list (unexpected but not fatal)
            };
            entry.profile_dht_key = Some(new_profile_dht_key.clone());

            let bytes = match serde_json::to_vec(&friend_list) {
                Ok(b) => b,
                Err(e) => {
                    tracing::warn!(error = %e, "friend list serialization failed during profile rotation");
                    return;
                }
            };

            if let Err(e) = self.io.write_record(
                &fl_key, 0, &bytes, Some(kp), crate::io::Confirm::Accepted,
            ).await {
                tracing::warn!(
                    peer = &sender_key[..12.min(sender_key.len())],
                    error = %e,
                    "friend list DHT update failed for profile rotation — \
                     next interaction with this peer may use stale profile key"
                );
            }
        }
    }
}
