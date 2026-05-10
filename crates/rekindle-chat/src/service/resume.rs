//! ChatService resume — reopen DHT records, publish route, set up watches.

use crate::crypto::SigningKeyHandle;
use crate::ChatError;

use super::ChatService;

impl ChatService {
    pub async fn resume(&self) -> Result<(), ChatError> {
        let handle = SigningKeyHandle::from_vault(&self.vault)?;
        self.io.set_signing_key(handle);

        let identity = {
            let meta = self.session_meta.read();
            meta.identity.clone().ok_or(ChatError::NotInitialized)?
        };

        let (_route_id, route_blob) = self.io.allocate_route().await?;

        // Reopen profile and publish route
        let profile_keypair = self.vault.load_key(
            rekindle_storage::keys::labels::PROFILE_KEYPAIR,
        )?;
        if let Some(ref kp) = profile_keypair {
            self.io.open_record(&identity.profile_dht_key, Some(kp)).await?;
            self.io.write_record(
                &identity.profile_dht_key,
                rekindle_types::dht_types::PROFILE_SUBKEY_ROUTE_BLOB,
                &route_blob, Some(kp), crate::io::Confirm::Accepted,
            ).await?;
            tracing::info!(
                profile = &identity.profile_dht_key[..12.min(identity.profile_dht_key.len())],
                "profile reopened + route published"
            );
        }

        // Reopen mailbox and publish route
        self.io.open_record(&identity.mailbox_dht_key, None).await?;
        let _ = self.io.write_record(
            &identity.mailbox_dht_key, 0, &route_blob,
            None, crate::io::Confirm::Accepted,
        ).await;

        // Reopen friend list
        let fl_kp = self.vault.load_key(
            rekindle_storage::keys::labels::FRIEND_LIST_KEYPAIR,
        )?;
        self.io.open_record(&identity.friend_list_dht_key, fl_kp.as_deref()).await?;

        // Reopen friend inbox + watch
        let fi_kp = self.vault.load_key(
            rekindle_storage::keys::labels::FRIEND_INBOX_KEYPAIR,
        )?;
        self.io.open_record(&identity.friend_inbox_key, fi_kp.as_deref()).await?;

        let inbox_subkeys: Vec<u32> = (0..32).collect();
        if let Err(e) = self.io.watch_and_register(
            &identity.friend_inbox_key, &inbox_subkeys,
            crate::events::registry::WatchKind::FriendInbox,
            &self.watches,
        ).await {
            tracing::warn!(error = %e, "friend inbox watch failed");
        }

        // Watch each DM peer's inbound log
        let dm_peers: Vec<(String, String)> = {
            let meta = self.session_meta.read();
            meta.dm_peers.iter()
                .filter(|(_, log)| !log.inbound_log_key.is_empty())
                .map(|(k, log)| (k.clone(), log.inbound_log_key.clone()))
                .collect()
        };
        for (peer_key, inbound_log_key) in &dm_peers {
            if let Err(e) = self.io.watch_and_register(
                inbound_log_key, &[0],
                crate::events::registry::WatchKind::DmLog { peer_key: peer_key.clone() },
                &self.watches,
            ).await {
                tracing::debug!(
                    peer = &peer_key[..12.min(peer_key.len())],
                    error = %e,
                    "DM watch failed"
                );
            }
        }

        // Open community records, set up watches, join meshes
        let communities: Vec<_> = {
            self.session_meta.read().communities.values().cloned().collect()
        };
        for m in &communities {
            if let Err(e) = self.io.open_record(&m.governance_key, None).await {
                tracing::warn!(community = %m.community_name, error = %e, "governance open failed");
            }
            if !m.registry_key.is_empty() {
                if let Err(e) = self.io.open_record(&m.registry_key, None).await {
                    tracing::warn!(community = %m.community_name, error = %e, "registry open failed");
                }
            }
            self.community.setup_community_watches(
                &m.governance_key, &m.registry_key, &m.join_inbox_key,
            ).await;
            if let Err(e) = self.io.join_mesh(&m.governance_key).await {
                tracing::warn!(community = %m.community_name, error = %e, "mesh join failed");
            }
        }

        tracing::info!(
            communities = communities.len(),
            dm_peers = self.session_meta.read().dm_peers.len(),
            "chat service resumed"
        );
        Ok(())
    }
}
