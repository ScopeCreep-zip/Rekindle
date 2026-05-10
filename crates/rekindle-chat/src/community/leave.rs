//! Community leave — write signed Leave entry to join inbox, broadcast
//! MemberLeave gossip, cancel watches, leave gossip mesh, clear MEK cache,
//! delete cached messages, update session meta.

use rekindle_types::dht_types::{PendingJoinEntry, PendingJoinStatus};
use rekindle_types::gossip_payload::{GossipPayload, ControlPayload};

use crate::io::Confirm;
use crate::time::timestamp_ms;
use crate::ChatError;
use super::CommunityService;

impl CommunityService {
    /// Leave a community. Writes a signed leave entry to the join inbox
    /// so the operator can process cleanup + rekey. Broadcasts MemberLeave
    /// gossip for real-time notification. Cleans up all local state.
    pub async fn leave_community(
        &self,
        governance_key: &str,
    ) -> Result<(), ChatError> {
        let membership = {
            let meta = self.session_meta.read();
            meta.communities.get(governance_key).cloned()
                .ok_or_else(|| ChatError::NotMember { community: governance_key.into() })?
        };

        let metadata = self.read_metadata(governance_key).await?;

        // Write signed Leave entry to join inbox
        if !metadata.join_inbox_key.is_empty() && !metadata.join_inbox_keypair_hex.is_empty() {
            let inbox_kp = hex::decode(&metadata.join_inbox_keypair_hex)
                .map_err(|e| ChatError::Internal(format!("inbox keypair hex: {e}")))?;

            if !inbox_kp.is_empty() {
                let pseudonym_seed = self.io.pseudonym_seed(governance_key)?;
                let kp = rekindle_ratchet::crypto::sign::keypair_from_seed(&pseudonym_seed)?;

                let now = timestamp_ms();
                let mut leave_entry = PendingJoinEntry {
                    requester_pseudonym_hex: membership.pseudonym_key.clone(),
                    display_name: membership.display_name.clone(),
                    profile_dht_key: String::new(),
                    x25519_pub_hex: String::new(),
                    invite_code_hash: None,
                    requested_at: now,
                    status: PendingJoinStatus::Left { left_at: now },
                    signature_hex: String::new(),
                };
                let content = leave_entry.signature_content();
                let sig = rekindle_ratchet::crypto::sign::sign_ec_prekey(&kp, &content);
                leave_entry.signature_hex = hex::encode(sig);

                // Read-append-write to preserve other entries in the subkey
                self.io.open_record(&metadata.join_inbox_key, Some(&inbox_kp)).await?;
                let subkey = blake3_subkey(&membership.pseudonym_key, 32);

                let existing = self.io.read_record(&metadata.join_inbox_key, subkey, true)
                    .await?
                    .unwrap_or_default();
                let mut entries: Vec<PendingJoinEntry> = if existing.is_empty() || existing == b"[]" {
                    Vec::new()
                } else {
                    serde_json::from_slice(&existing).unwrap_or_default()
                };
                entries.retain(|e| e.requester_pseudonym_hex != leave_entry.requester_pseudonym_hex);
                entries.push(leave_entry);

                let bytes = serde_json::to_vec(&entries)
                    .map_err(|e| ChatError::Serialization(format!("leave entry: {e}")))?;

                match self.io.write_record(
                    &metadata.join_inbox_key, subkey, &bytes,
                    Some(&inbox_kp), Confirm::Accepted,
                ).await {
                    Ok(_) => tracing::info!(
                        community = %metadata.name,
                        "signed leave entry written to join inbox"
                    ),
                    Err(e) => tracing::warn!(
                        community = %metadata.name,
                        error = %e,
                        "leave entry write failed — operator will not receive cleanup signal. \
                         Community MEKs will NOT be rotated for forward secrecy."
                    ),
                }
            }
        }

        // Broadcast MemberLeave gossip for real-time notification
        if let Err(e) = self.io.broadcast_gossip_dedup(
            governance_key,
            GossipPayload::Control(ControlPayload::MemberLeave {
                pseudonym_key: membership.pseudonym_key.clone(),
            }),
        ).await {
            tracing::debug!(
                community = %metadata.name,
                error = %e,
                "MemberLeave gossip failed — operator will discover via inbox poll"
            );
        }

        // Best-effort RPC notification to operator via direct gossip
        if !metadata.owner_pseudonym.is_empty() {
            if let Err(e) = self.io.send_gossip_direct(
                governance_key,
                &metadata.owner_pseudonym,
                GossipPayload::Control(ControlPayload::MemberLeave {
                    pseudonym_key: membership.pseudonym_key.clone(),
                }),
            ).await {
                tracing::debug!(
                    error = %e,
                    "direct leave notification to operator failed — operator will discover via inbox"
                );
            }
        }

        // Cancel watches for this community
        if let Some(token) = self.watches.unregister(governance_key) {
            if let Err(e) = self.io.cancel_watch(token).await {
                tracing::debug!(error = %e, "governance watch cancel failed");
            }
        }
        if !membership.registry_key.is_empty() {
            if let Some(token) = self.watches.unregister(&membership.registry_key) {
                if let Err(e) = self.io.cancel_watch(token).await {
                    tracing::debug!(error = %e, "registry watch cancel failed");
                }
            }
        }
        if !membership.join_inbox_key.is_empty() {
            if let Some(token) = self.watches.unregister(&membership.join_inbox_key) {
                if let Err(e) = self.io.cancel_watch(token).await {
                    tracing::debug!(error = %e, "join inbox watch cancel failed");
                }
            }
        }

        // Leave gossip mesh
        if let Err(e) = self.io.leave_mesh(governance_key).await {
            tracing::debug!(error = %e, "gossip mesh leave failed");
        }

        // Clear MEK cache for this community
        self.mek_cache.remove_community(governance_key);

        // Remove from session meta
        {
            let mut meta = self.session_meta.write();
            meta.communities.remove(governance_key);
        }

        // Delete cached channel messages from vault
        if let Err(e) = self.vault.delete_community_messages(governance_key) {
            tracing::warn!(error = %e, "cached message cleanup failed — stale messages may remain in vault");
        }

        tracing::info!(
            community = %metadata.name,
            governance = &governance_key[..12.min(governance_key.len())],
            "left community — all local state cleared"
        );

        Ok(())
    }
}

fn blake3_subkey(input: &str, n: u32) -> u32 {
    let hash = blake3::hash(input.as_bytes());
    let bytes = hash.as_bytes();
    u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) % n
}