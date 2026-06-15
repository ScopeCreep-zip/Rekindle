//! ChatService resume — reopen DHT records, publish route, set up watches.

use zeroize::Zeroizing;
use rekindle_identity::self_id::SelfIdentity;
use rekindle_storage::keys::labels;
use rekindle_types::dm_store::DmStore;

use crate::ChatError;

use super::ChatService;

impl ChatService {
    pub async fn resume(&self) -> Result<(), ChatError> {
        // Load signing seed from vault, construct SelfIdentity
        let seed_bytes = self.vault.require_key(labels::SIGNING_KEY)?;
        if seed_bytes.len() != 32 {
            return Err(ChatError::Internal(format!(
                "signing key wrong length: {} (expected 32)", seed_bytes.len()
            )));
        }
        let mut seed_arr = Zeroizing::new([0u8; 32]);
        seed_arr.copy_from_slice(&seed_bytes);

        let identity = {
            let meta = self.session_meta.read();
            meta.identity.clone().ok_or(ChatError::NotInitialized)?
        };

        let self_id = SelfIdentity::restore_from_seed(
            seed_arr,
            &identity.profile_dht_key,
        ).map_err(|e| ChatError::Internal(format!("identity restore: {e}")))?;

        self.io.set_identity(self_id);

        let (_route_id, route_blob) = self.io.allocate_route().await?;

        // Reopen profile and publish route
        let profile_keypair = self.vault.load_key(
            rekindle_storage::keys::labels::PROFILE_KEYPAIR,
        )?;
        if let Some(ref kp) = profile_keypair {
            let profile_record = self.io.open_record(&identity.profile_dht_key, Some(kp)).await?;
            self.io.write_record(
                &profile_record,
                rekindle_types::dht_types::PROFILE_SUBKEY_ROUTE_BLOB,
                &route_blob, Some(kp), crate::io::Confirm::Accepted,
            ).await?;
            // Publish X25519 DH public key for group DM MEK wrapping
            let x25519_seed = self.io.x25519_identity_seed()?;
            let x25519_pub = rekindle_identity::x25519_public_from_raw_seed(&x25519_seed)
                .map_err(|e| ChatError::Internal(format!("x25519 pub derive: {e}")))?;
            self.io.write_record(
                &profile_record,
                rekindle_types::dht_types::PROFILE_SUBKEY_X25519_PUB,
                x25519_pub.as_bytes(), Some(kp), crate::io::Confirm::Accepted,
            ).await?;

            tracing::info!(
                profile = &identity.profile_dht_key[..12.min(identity.profile_dht_key.len())],
                "profile reopened + route + X25519 pub published"
            );
        }

        // Reopen mailbox and publish route
        let _ = self.io.open_and_write(
            &identity.mailbox_dht_key, 0, &route_blob,
            None, crate::io::Confirm::Accepted,
        ).await;

        // Reopen friend inbox + watch
        let fi_kp = self.vault.load_key(
            rekindle_storage::keys::labels::FRIEND_INBOX_KEYPAIR,
        )?;
        let fi_record = self.io.open_record(&identity.friend_inbox_key, fi_kp.as_deref()).await?;

        let inbox_subkeys: Vec<u32> = (0..32).collect();
        if let Err(e) = self.io.watch_and_register(
            &fi_record, &inbox_subkeys,
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
            if let Err(e) = self.io.open_and_watch(
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

        // Reopen SMPL DM records and re-establish watches
        let dm_smpl: Vec<(String, String)> = {
            let meta = self.session_meta.read();
            meta.dm_smpl_peers.iter()
                .map(|(k, p)| (k.clone(), p.record_key.clone()))
                .collect()
        };
        for (peer_key, record_key) in &dm_smpl {
            let smpl_record = match self.io.open_record(record_key, None).await {
                Ok(r) => r,
                Err(e) => {
                    tracing::debug!(
                        peer = &peer_key[..12.min(peer_key.len())],
                        error = %e,
                        "resume: SMPL DM open failed"
                    );
                    continue;
                }
            };
            let peer_subkey = match self.vault.dm_get_session_meta(record_key) {
                Ok(Some(meta)) => {
                    let peer_sk = if meta.my_subkey == 0 { 1u32 } else { 0u32 };
                    tracing::debug!(
                        peer = &peer_key[..12.min(peer_key.len())],
                        my_subkey = meta.my_subkey,
                        peer_subkey = peer_sk,
                        "resume: SMPL session meta loaded"
                    );
                    peer_sk
                }
                Ok(None) => {
                    tracing::warn!(
                        peer = &peer_key[..12.min(peer_key.len())],
                        record_key = &record_key[..12.min(record_key.len())],
                        "resume: no session meta in vault — defaulting peer_subkey=1"
                    );
                    1
                }
                Err(e) => {
                    tracing::warn!(
                        peer = &peer_key[..12.min(peer_key.len())],
                        error = %e,
                        "resume: session meta load error — defaulting peer_subkey=1"
                    );
                    1
                }
            };
            if let Err(e) = self.io.watch_and_register(
                &smpl_record,
                &[peer_subkey],
                crate::events::registry::WatchKind::DmSmpl { record_key: record_key.clone() },
                &self.watches,
            ).await {
                tracing::debug!(
                    peer = &peer_key[..12.min(peer_key.len())],
                    error = %e,
                    "resume: SMPL DM watch failed"
                );
            }
        }
        if !dm_smpl.is_empty() {
            tracing::info!(dm_smpl = dm_smpl.len(), "resume: SMPL DM records reopened");
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

            // Populate gossip mesh via transport's MeshManager.
            // Reads member registry, resolves routes via DHT, caches in PeerRegistry,
            // upserts into gossip mesh. All internal to the transport layer.
            let members = self.community.list_members(&m.governance_key).await.unwrap_or_default();
            let member_pairs: Vec<(String, Option<String>)> = members.iter()
                .map(|mem| (mem.pseudonym_key.clone(), mem.profile_dht_key.clone()))
                .collect();
            self.io.transport().populate_mesh(&m.governance_key, &m.pseudonym_key, &member_pairs).await;
        }

        // Slow-path catch-up: read messages missed while offline
        let caught = self.catch_up_channel_messages().await;
        if caught > 0 {
            tracing::info!(caught, "resume: caught up missed channel messages via slow path");
        }

        let dm_caught = self.catch_up_dm_messages().await;
        if dm_caught > 0 {
            tracing::info!(dm_caught, "resume: caught up missed DM messages via slow path");
        }

        // Register friend peer→profile mappings and discover routes
        {
            let friend_profiles: Vec<(String, String)> = {
                let meta = self.session_meta.read();
                meta.pending_friend_requests.iter()
                    .map(|r| (r.sender_public_key.clone(), r.profile_dht_key.clone()))
                    .collect()
            };
            for (peer_key, profile_key) in &friend_profiles {
                self.io.transport().register_peer_profile(peer_key, &profile_key);
            }
        }

        tracing::info!(
            communities = communities.len(),
            dm_peers = self.session_meta.read().dm_peers.len(),
            peer_count = self.io.transport().peer_count(),
            mesh_peers = self.io.transport().gossip_mesh_peer_count(),
            "chat service resumed"
        );
        Ok(())
    }
}
