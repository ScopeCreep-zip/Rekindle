//! Remove a friend — delete from dm_peers, notify via PlatformIO.

use rekindle_types::dm_payload::DmPayload;

use crate::io::Confirm;
use crate::ChatError;
use super::FriendshipService;

impl FriendshipService {
    /// Remove a friend. Sends Unfriend notification, cancels DM watch,
    /// removes from session meta, invalidates route cache.
    pub async fn remove_friend(
        &self,
        peer_pubkey: &str,
    ) -> Result<(), ChatError> {
        // Send Unfriend notification via PlatformIO (best-effort)
        if let Err(e) = self.io.send_peer_notification(
            peer_pubkey,
            DmPayload::Unfriend,
            Confirm::None,
        ).await {
            tracing::debug!(
                peer = &peer_pubkey[..12.min(peer_pubkey.len())],
                error = %e,
                "unfriend notification failed — peer may not know they were removed"
            );
        }

        // Cancel DM watch if exists
        let inbound_key = {
            let meta = self.session_meta.read();
            meta.dm_peers
                .get(peer_pubkey)
                .map(|p| p.inbound_log_key.clone())
                .unwrap_or_default()
        };
        if !inbound_key.is_empty() {
            if let Some(token) = self.watches.unregister(&inbound_key) {
                if let Err(e) = self.io.cancel_watch(token).await {
                    tracing::debug!(error = %e, "DM watch cancel failed during friend removal");
                }
            }
        }

        // Remove from session meta
        {
            let mut meta = self.session_meta.write();
            meta.dm_peers.remove(peer_pubkey);
            meta.friend_display_names.remove(peer_pubkey);
        }

        // Invalidate cached route
        self.io.invalidate_peer_route(peer_pubkey);

        // Delete friend name from vault
        let _ = self.vault.delete_friend_name(peer_pubkey);

        tracing::info!(
            peer = &peer_pubkey[..12.min(peer_pubkey.len())],
            "friend removed"
        );

        Ok(())
    }
}
