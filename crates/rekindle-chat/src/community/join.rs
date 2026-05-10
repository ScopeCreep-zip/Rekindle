//! Community join — 3-phase lifecycle: submit request, await approval,
//! complete with MEK cache warming + watch setup + gossip mesh join.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use rekindle_types::dht_types::{
    CommunityMetadata, MemberSummary, MekVaultEntry, PendingJoinEntry,
    PendingJoinStatus, MANIFEST_METADATA, MANIFEST_CHANNELS,
    MANIFEST_REGISTRY_SPINE, REGISTRY_MEMBER_INDEX, REGISTRY_MEK_VAULT,
};
use rekindle_types::session_types::CommunityMembership;
use rekindle_types::transport::RecordSchema;

use crate::events::registry::WatchKind;
use crate::io::Confirm;
use crate::time::timestamp_ms;
use crate::ChatError;
use super::CommunityService;

// ── Result types ────────────────────────────────────────────────────

pub struct JoinRequestSubmitted {
    pub community_name: String,
    pub governance_key: String,
    pub pseudonym_hex: String,
    pub registry_key: String,
    pub community_mailbox_key: String,
    pub join_inbox_key: String,
}

pub struct JoinApproved {
    pub slot_index: u32,
    pub discovery_tier: &'static str,
    pub elapsed: Duration,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct JoinCompleted {
    pub community_name: String,
    pub governance_key: String,
    pub pseudonym_hex: String,
    pub slot_index: u32,
    pub registry_key: String,
    pub community_mailbox_key: String,
    pub channels_discovered: usize,
    pub meks_cached: usize,
}

impl CommunityService {
    // ── Phase 1: Submit ─────────────────────────────────────────

    /// Submit a join request to a community's inbox.
    ///
    /// Reads governance metadata, derives pseudonym + X25519 key, signs
    /// the join entry, writes to the inbox with Confirm::Verified.
    /// Returns immediately — approval is discovered in phase 2.
    pub async fn submit_join_request(
        &self,
        governance_key: &str,
        display_name: &str,
    ) -> Result<JoinRequestSubmitted, ChatError> {
        // Read governance metadata
        self.io.open_record(governance_key, None).await?;
        let metadata = self.read_governance_metadata(governance_key).await?;

        if metadata.join_inbox_key.is_empty() || metadata.join_inbox_keypair_hex.is_empty() {
            // Retry with force refresh — the record may not have propagated yet
            let metadata = self.read_governance_metadata_fresh(governance_key).await?;
            if metadata.join_inbox_key.is_empty() || metadata.join_inbox_keypair_hex.is_empty() {
                return Err(ChatError::Internal(format!(
                    "community has no join inbox — community may not be fully created yet \
                     (governance {})",
                    &governance_key[..12.min(governance_key.len())],
                )));
            }
        }

        // Read registry key from governance spine
        let registry_key = self.read_registry_key(governance_key, &metadata.name).await?;

        // Derive pseudonym
        let pseudonym_hex = self.io.pseudonym_hex(governance_key)?;
        let pseudonym_seed = self.io.pseudonym_seed(governance_key)?;
        let kp = rekindle_ratchet::crypto::sign::keypair_from_seed(&pseudonym_seed)?;

        // Derive X25519 public key for MEK wrapping
        let community_x25519_seed = blake3::derive_key("rekindle identity x25519 v1", &pseudonym_seed);
        let x25519_key = rekindle_ratchet::crypto::dh::reusable_from_seed(&community_x25519_seed)
            .map_err(|e| ChatError::Internal(format!("x25519 from seed: {e}")))?;
        let x25519_pub_raw = x25519_key.compute_public_key()
            .map_err(|_| ChatError::Internal("x25519 pubkey derive".into()))?;
        let x25519_pub_hex = hex::encode(x25519_pub_raw.as_ref());

        // Build + sign join entry
        let mut entry = PendingJoinEntry {
            requester_pseudonym_hex: pseudonym_hex.clone(),
            display_name: display_name.to_string(),
            profile_dht_key: {
                let meta = self.session_meta.read();
                meta.identity.as_ref().map(|i| i.profile_dht_key.clone()).unwrap_or_default()
            },
            x25519_pub_hex,
            invite_code_hash: None,
            requested_at: timestamp_ms(),
            status: PendingJoinStatus::Pending,
            signature_hex: String::new(),
        };
        let content = entry.signature_content();
        let sig = rekindle_ratchet::crypto::sign::sign_ec_prekey(&kp, &content);
        entry.signature_hex = hex::encode(sig);

        // Write to join inbox with Confirm::Verified
        let inbox_kp_bytes = hex::decode(&metadata.join_inbox_keypair_hex)
            .map_err(|e| ChatError::Internal(format!("inbox keypair hex: {e}")))?;
        self.io.open_record(&metadata.join_inbox_key, Some(&inbox_kp_bytes)).await?;

        let subkey = blake3_hash_mod(&pseudonym_hex, governance_key, 32);
        let existing = self.io.read_record(&metadata.join_inbox_key, subkey, true)
            .await?
            .unwrap_or_default();

        let mut entries: Vec<PendingJoinEntry> = if existing.is_empty() || existing == b"[]" {
            Vec::new()
        } else {
            serde_json::from_slice(&existing).unwrap_or_default()
        };
        entries.retain(|e| e.requester_pseudonym_hex != entry.requester_pseudonym_hex);
        entries.push(entry);

        let bytes = serde_json::to_vec(&entries)
            .map_err(|e| ChatError::Serialization(format!("join entry: {e}")))?;

        let receipt = self.io.write_record(
            &metadata.join_inbox_key, subkey, &bytes,
            Some(&inbox_kp_bytes), Confirm::Verified,
        ).await?;

        if receipt.verified {
            tracing::info!(
                community = %metadata.name,
                governance = &governance_key[..12.min(governance_key.len())],
                elapsed_ms = receipt.elapsed.as_millis(),
                "join request submitted and verified"
            );
        } else {
            tracing::warn!(
                community = %metadata.name,
                governance = &governance_key[..12.min(governance_key.len())],
                "join request submitted but verification inconclusive — \
                 operator may experience delay discovering the request"
            );
        }

        // Best-effort direct notification to operator
        if let Err(e) = self.io.send_peer_notification(
            &metadata.owner_pseudonym,
            rekindle_types::dm_payload::DmPayload::FriendRequestAck,
            Confirm::None,
        ).await {
            tracing::debug!(
                error = %e,
                "join notification to operator failed — operator will discover via inbox poll"
            );
        }

        Ok(JoinRequestSubmitted {
            community_name: metadata.name,
            governance_key: governance_key.to_string(),
            pseudonym_hex,
            registry_key,
            community_mailbox_key: metadata.community_mailbox_key,
            join_inbox_key: metadata.join_inbox_key,
        })
    }

    // ── Phase 2: Await approval ─────────────────────────────────

    /// Wait for the operator to approve our join request.
    ///
    /// Polls the member registry every 5 seconds until our pseudonym
    /// appears. Returns when approved or when timeout expires.
    ///
    /// In production, a watch on the registry record and a gossip
    /// JoinAccepted notification provide faster discovery — but poll
    /// is the reliable fallback that always works.
    pub async fn await_join_approval(
        &self,
        submitted: &JoinRequestSubmitted,
        timeout_secs: u64,
    ) -> Result<JoinApproved, ChatError> {
        let poll_interval = Duration::from_secs(5);
        let deadline = Duration::from_secs(timeout_secs);
        let start = Instant::now();

        tracing::info!(
            community = %submitted.community_name,
            timeout_secs,
            "awaiting join approval — polling registry every 5s"
        );

        // Establish registry watch for faster discovery
        if let Err(e) = self.io.watch_and_register(
            &submitted.registry_key, &[REGISTRY_MEMBER_INDEX],
            WatchKind::MemberRegistry { community: submitted.governance_key.clone() },
            &self.watches,
        ).await {
            tracing::debug!(error = %e, "registry watch for join approval failed — relying on poll");
        }

        loop {
            if start.elapsed() >= deadline {
                return Err(ChatError::Internal(format!(
                    "join not approved within {timeout_secs}s for community '{}' — \
                     the community operator may be offline, the community may use \
                     manual approval (WaitingRoom policy), or DHT propagation is slow. \
                     Your request is persisted in the community's inbox — approval \
                     will be discovered on your next daemon startup.",
                    submitted.community_name,
                )));
            }

            // Poll registry for our pseudonym
            let members_data = self.io.read_record(
                &submitted.registry_key, REGISTRY_MEMBER_INDEX, true,
            ).await?;

            if let Some(data) = members_data {
                let members: Vec<MemberSummary> = serde_json::from_slice(&data).unwrap_or_default();
                if let Some(m) = members.iter().find(|m| m.pseudonym_key == submitted.pseudonym_hex) {
                    let elapsed = start.elapsed();
                    tracing::info!(
                        community = %submitted.community_name,
                        slot = m.subkey_index,
                        elapsed_secs = elapsed.as_secs(),
                        "join approved — discovered via registry poll"
                    );
                    return Ok(JoinApproved {
                        slot_index: m.subkey_index,
                        discovery_tier: "poll",
                        elapsed,
                    });
                }
            }

            tokio::time::sleep(poll_interval).await;
        }
    }

    // ── Phase 3: Complete ───────────────────────────────────────

    /// Complete the join after approval — MEK cache warming, slot seed
    /// derivation, watch setup, gossip mesh join, session meta update.
    pub async fn complete_join(
        &self,
        submitted: &JoinRequestSubmitted,
        approved: &JoinApproved,
    ) -> Result<JoinCompleted, ChatError> {
        // Read channel list
        let channels_data = self.io.read_record(
            &submitted.governance_key, MANIFEST_CHANNELS, true,
        ).await?;
        let channels: Vec<rekindle_types::dht_types::ChannelEntry> = channels_data
            .and_then(|d| serde_json::from_slice(&d).ok())
            .unwrap_or_default();
        let channels_discovered = channels.len();

        // Read and unwrap MEKs
        let meks_cached = self.warm_mek_cache(
            &submitted.governance_key,
            &submitted.registry_key,
            &submitted.pseudonym_hex,
        ).await;

        // Derive and store SMPL slot seed
        let signing_seed = self.io.require_signing_key()?;
        let slot_seed = blake3::derive_key(
            &format!("rekindle slot seed v1 {} {}", submitted.governance_key, approved.slot_index),
            &signing_seed,
        );
        let gov_short = &submitted.governance_key[..12.min(submitted.governance_key.len())];
        self.vault.store_key(
            &format!("community.slot.{gov_short}.{}", approved.slot_index),
            &slot_seed,
        )?;

        // Create per-channel DhtLog records so we can write messages to channels.
        // Each member owns their own DhtLog per channel — no shared write access.
        let mut channel_record_keys = HashMap::new();
        for ch in &channels {
            match self.io.create_record(RecordSchema::SingleWriter { subkey_count: 1 }).await {
                Ok((record_key, keypair)) => {
                    let ch_short = &record_key[..12.min(record_key.len())];
                    if let Err(e) = self.vault.store_key(
                        &rekindle_storage::keys::labels::channel_log_keypair(ch_short),
                        &keypair,
                    ) {
                        tracing::warn!(channel = %ch.id, error = %e, "channel log keypair vault store failed");
                        continue;
                    }
                    channel_record_keys.insert(ch.id.clone(), record_key);
                }
                Err(e) => {
                    tracing::warn!(
                        channel = %ch.id,
                        error = %e,
                        "per-channel DhtLog creation failed — will retry on next message send"
                    );
                }
            }
        }

        // Channel record registration with the operator happens via
        // GovernanceOp::RegisterChannelRecord RPC when the member first
        // sends a message to a channel. The RPC carries the record key
        // so the operator can update the member registry. This is deferred
        // to first-message-send to avoid RPC overhead during join.

        // Establish subscription watches
        self.setup_community_watches(
            &submitted.governance_key,
            &submitted.registry_key,
            &submitted.join_inbox_key,
        ).await;

        // Join gossip mesh
        if let Err(e) = self.io.join_mesh(&submitted.governance_key).await {
            tracing::warn!(
                community = %submitted.community_name,
                error = %e,
                "gossip mesh join failed — real-time updates will be slower (watch+poll only)"
            );
        }

        // Update session meta
        let display_name = {
            let meta = self.session_meta.read();
            meta.identity.as_ref().map(|i| i.display_name.clone()).unwrap_or_default()
        };
        {
            let mut meta = self.session_meta.write();
            meta.communities.insert(submitted.governance_key.clone(), CommunityMembership {
                community_name: submitted.community_name.clone(),
                governance_key: submitted.governance_key.clone(),
                registry_key: submitted.registry_key.clone(),
                pseudonym_key: submitted.pseudonym_hex.clone(),
                display_name: display_name.clone(),
                role_ids: Vec::new(),
                slot_index: approved.slot_index,
                channel_record_keys,
                community_mailbox_key: submitted.community_mailbox_key.clone(),
                join_inbox_key: submitted.join_inbox_key.clone(),
                is_operator: false,
                locked_down: false,
                joined_at: timestamp_ms(),
            });
        }

        tracing::info!(
            community = %submitted.community_name,
            slot = approved.slot_index,
            channels = channels_discovered,
            meks = meks_cached,
            "join completed — watches + gossip mesh active"
        );

        Ok(JoinCompleted {
            community_name: submitted.community_name.clone(),
            governance_key: submitted.governance_key.clone(),
            pseudonym_hex: submitted.pseudonym_hex.clone(),
            slot_index: approved.slot_index,
            registry_key: submitted.registry_key.clone(),
            community_mailbox_key: submitted.community_mailbox_key.clone(),
            channels_discovered,
            meks_cached,
        })
    }

    // ── Convenience wrapper ─────────────────────────────────────

    /// Join a community — submit → await (120s) → complete.
    pub async fn join_community(
        &self,
        governance_key: &str,
    ) -> Result<JoinCompleted, ChatError> {
        let display_name = {
            let meta = self.session_meta.read();
            meta.identity.as_ref().map(|i| i.display_name.clone()).unwrap_or_default()
        };
        let submitted = self.submit_join_request(governance_key, &display_name).await?;
        let approved = self.await_join_approval(&submitted, 120).await?;
        self.complete_join(&submitted, &approved).await
    }

    // ── Internal helpers ────────────────────────────────────────

    async fn read_governance_metadata(
        &self, governance_key: &str,
    ) -> Result<CommunityMetadata, ChatError> {
        let raw = self.io.read_record(governance_key, MANIFEST_METADATA, false).await?
            .ok_or_else(|| ChatError::CommunityNotFound { community: governance_key.into() })?;
        serde_json::from_slice(&raw)
            .map_err(|e| ChatError::Deserialization(format!("governance metadata: {e}")))
    }

    async fn read_governance_metadata_fresh(
        &self, governance_key: &str,
    ) -> Result<CommunityMetadata, ChatError> {
        let raw = self.io.read_record(governance_key, MANIFEST_METADATA, true).await?
            .ok_or_else(|| ChatError::CommunityNotFound { community: governance_key.into() })?;
        serde_json::from_slice(&raw)
            .map_err(|e| ChatError::Deserialization(format!("governance metadata: {e}")))
    }

    async fn read_registry_key(
        &self, governance_key: &str, community_name: &str,
    ) -> Result<String, ChatError> {
        let raw = self.io.read_record(governance_key, MANIFEST_REGISTRY_SPINE, true).await?
            .ok_or_else(|| ChatError::Internal(format!(
                "community '{community_name}' has no registry spine — community may be corrupted"
            )))?;
        let spine: serde_json::Value = serde_json::from_slice(&raw)
            .map_err(|e| ChatError::Deserialization(format!("registry spine: {e}")))?;
        spine.get("primary_key")
            .and_then(serde_json::Value::as_str)
            .map(String::from)
            .ok_or_else(|| ChatError::Internal(format!(
                "registry spine missing 'primary_key' for community '{community_name}'"
            )))
    }

    /// Read MEK vault, find copies for our pseudonym, unwrap via ECDH, cache.
    /// Returns the number of MEKs successfully cached.
    async fn warm_mek_cache(
        &self,
        governance_key: &str,
        registry_key: &str,
        our_pseudonym_hex: &str,
    ) -> usize {
        let Some(vault) = self.read_mek_vault_with_retry(registry_key, our_pseudonym_hex).await else {
            tracing::warn!(
                governance = &governance_key[..12.min(governance_key.len())],
                "MEK vault has no copies for us after retry — channels will be unreadable \
                 until MEKs are received via gossip MekTransfer"
            );
            return 0;
        };

        let pseudonym_seed = match self.io.pseudonym_seed(governance_key) {
            Ok(s) => s,
            Err(e) => {
                tracing::error!(error = %e, "cannot derive pseudonym seed for MEK unwrap");
                return 0;
            }
        };
        let our_x25519_seed = blake3::derive_key("rekindle identity x25519 v1", &pseudonym_seed);

        let mut cached = 0usize;
        for entry in &vault {
            let Some(copy) = entry.copies.iter().find(|c| c.target_pseudonym == our_pseudonym_hex) else {
                continue;
            };

            let Some(rotator_pub) = hex::decode(&entry.rotator_pseudonym)
                .ok()
                .and_then(|b| <[u8; 32]>::try_from(b.as_slice()).ok()) else {
                tracing::warn!(
                    channel = %entry.channel_id,
                    rotator = &entry.rotator_pseudonym[..12.min(entry.rotator_pseudonym.len())],
                    "invalid rotator pseudonym hex — skipping MEK"
                );
                continue;
            };

            match crate::crypto::mek::unwrap_mek(&our_x25519_seed, &rotator_pub, &copy.encrypted_mek) {
                Ok(mek_wire) => {
                    match crate::crypto::mek::mek_from_wire(&mek_wire) {
                        Ok((key, generation)) => {
                            self.mek_cache.insert(governance_key, &entry.channel_id, key, generation);
                            cached += 1;
                        }
                        Err(e) => {
                            tracing::error!(
                                channel = %entry.channel_id,
                                generation = entry.generation,
                                error = %e,
                                "MEK wire format invalid after unwrap — channel will be unreadable"
                            );
                        }
                    }
                }
                Err(e) => {
                    tracing::error!(
                        channel = %entry.channel_id,
                        generation = entry.generation,
                        rotator = &entry.rotator_pseudonym[..12.min(entry.rotator_pseudonym.len())],
                        error = %e,
                        "MEK ECDH unwrap FAILED — channel will be unreadable. \
                         Operator must re-rotate MEK for this channel."
                    );
                }
            }
        }

        tracing::info!(cached, total_entries = vault.len(), "MEK cache warmed");
        cached
    }

    /// Read MEK vault with retry — the operator may not have finished wrapping
    /// MEKs for the new member yet.
    async fn read_mek_vault_with_retry(
        &self,
        registry_key: &str,
        our_pseudonym_hex: &str,
    ) -> Option<Vec<MekVaultEntry>> {
        let mut backoff = Duration::from_secs(2);
        let ceiling = Duration::from_secs(5);
        let deadline = Duration::from_secs(20);
        let start = Instant::now();

        loop {
            let data = self.io.read_record(registry_key, REGISTRY_MEK_VAULT, true)
                .await
                .ok()?;

            if let Some(bytes) = data {
                let vault: Vec<MekVaultEntry> = serde_json::from_slice(&bytes).unwrap_or_default();
                if vault.iter().any(|e| e.copies.iter().any(|c| c.target_pseudonym == our_pseudonym_hex)) {
                    return Some(vault);
                }
            }

            if start.elapsed() >= deadline {
                return None;
            }

            tracing::debug!(
                elapsed_secs = start.elapsed().as_secs(),
                backoff_ms = backoff.as_millis(),
                "MEK vault has no copies for us yet — retrying"
            );
            tokio::time::sleep(backoff).await;
            backoff = (backoff * 2).min(ceiling);
        }
    }

    /// Set up subscription watches for a community.
    pub(crate) async fn setup_community_watches(
        &self,
        governance_key: &str,
        registry_key: &str,
        join_inbox_key: &str,
    ) {
        // Governance manifest (channels, roles, bans, invites, metadata, social subkeys)
        if let Err(e) = self.io.watch_and_register(
            governance_key, &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16],
            WatchKind::GovernanceManifest { community: governance_key.to_string() },
            &self.watches,
        ).await {
            tracing::warn!(
                governance = &governance_key[..12.min(governance_key.len())],
                error = %e,
                "governance watch failed — updates will arrive via poll only"
            );
        }

        // Member registry
        if let Err(e) = self.io.watch_and_register(
            registry_key, &[REGISTRY_MEMBER_INDEX],
            WatchKind::MemberRegistry { community: governance_key.to_string() },
            &self.watches,
        ).await {
            tracing::warn!(
                registry = &registry_key[..12.min(registry_key.len())],
                error = %e,
                "registry watch failed — member changes will arrive via poll only"
            );
        }

        // Join inbox (operators only — but set up for all, harmless if not operator)
        let inbox_subkeys: Vec<u32> = (0..32).collect();
        if let Err(e) = self.io.watch_and_register(
            join_inbox_key, &inbox_subkeys,
            WatchKind::JoinInbox { community: governance_key.to_string() },
            &self.watches,
        ).await {
            tracing::debug!(
                error = %e,
                "join inbox watch failed — inbox changes will arrive via poll only"
            );
        }
    }
}

/// Deterministic subkey index from two keys.
fn blake3_hash_mod(a: &str, b: &str, n: u32) -> u32 {
    let mut hasher = blake3::Hasher::new();
    hasher.update(a.as_bytes());
    hasher.update(b"|");
    hasher.update(b.as_bytes());
    let hash = hasher.finalize();
    let bytes = hash.as_bytes();
    u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) % n
}
