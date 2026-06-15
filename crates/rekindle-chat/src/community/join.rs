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

        // Read registry key from governance spine, then open it.
        // The registry is a separate DHT record from governance — it must
        // be opened before any read_record call (await_join_approval polls
        // REGISTRY_MEMBER_INDEX, complete_join reads REGISTRY_MEK_VAULT).
        let registry_key = self.read_registry_key(governance_key, &metadata.name).await?;

        // max_members enforcement
        let current_members = self.read_members(&registry_key).await.map_or(0, |m| m.len());
        if current_members >= metadata.max_members as usize {
            return Err(ChatError::CommunityFull {
                current: current_members,
                max: metadata.max_members,
            });
        }

        // Derive pseudonym via identity crate (governance key canonicalized)
        let pseudonym_hex = self.io.pseudonym_hex(governance_key)?;
        let pseudonym_seed = self.io.pseudonym_seed(governance_key)?;
        let kp = rekindle_identity::SigningKeypair::from_seed(&pseudonym_seed)
            .map_err(|e| ChatError::Internal(format!("pseudonym keypair: {e}")))?;

        // Derive X25519 public key for MEK wrapping (G5 from pseudonym seed)
        let community_x25519_seed = rekindle_identity::x25519_seed_from(&pseudonym_seed);
        let our_community_dh = rekindle_identity::x25519_public_from_raw_seed(&community_x25519_seed)
            .map_err(|e| ChatError::Internal(format!("x25519 from seed: {e}")))?;
        let x25519_pub_hex = our_community_dh.to_hex();

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
        let sig = kp.sign_ec_prekey(&content);
        entry.signature_hex = hex::encode(sig);

        // Write to join inbox with Confirm::Verified
        let inbox_kp_bytes = hex::decode(&metadata.join_inbox_keypair_hex)
            .map_err(|e| ChatError::Internal(format!("inbox keypair hex: {e}")))?;
        let inbox_record = self.io.open_record(&metadata.join_inbox_key, Some(&inbox_kp_bytes)).await?;

        let subkey = blake3_hash_mod(&pseudonym_hex, governance_key, 32);
        let existing = self.io.read_record(&inbox_record, subkey, true)
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
            &inbox_record, subkey, &bytes,
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
        if let Err(e) = self.io.open_and_watch(
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
            let members_data = self.io.open_and_read(
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
        let channels_data = self.read_verified_governance(
            &submitted.governance_key, MANIFEST_CHANNELS,
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

        // Build channel name→UUID resolution map and slowmode config from governance channel list.
        let mut channel_name_to_id = HashMap::new();
        let mut channel_slowmode = HashMap::new();
        for ch in &channels {
            channel_name_to_id.insert(ch.name.clone(), ch.id.clone());
            channel_slowmode.insert(ch.id.clone(), ch.slowmode_seconds);
        }

        // Unwrap slot_seed from MEK vault (ECDH-encrypted per member)
        let metadata = self.read_governance_metadata(&submitted.governance_key).await?;
        let slot_seed = self.unwrap_slot_seed_from_vault(
            &submitted.governance_key,
            &submitted.pseudonym_hex,
        ).await.or_else(|_| {
            // Fallback: read from metadata (operator-only, plaintext)
            hex::decode(&metadata.slot_seed_hex)
                .ok()
                .and_then(|b| <[u8; 32]>::try_from(b.as_slice()).ok())
                .ok_or_else(|| ChatError::Internal("slot_seed not available from vault or metadata".into()))
        })?;

        let mut channel_record_keys = HashMap::new();

        // Derive our slot keypair for writing to channel SMPL records
        let slot_keypair = rekindle_identity::derive_slot_keypair(&slot_seed, approved.slot_index)
            .map_err(|e| ChatError::Internal(format!("slot keypair derivation: {e}")))?;
        let slot_keypair_bytes = {
            let mut buf = [0u8; 64];
            buf[..32].copy_from_slice(&slot_keypair.public_key_bytes());
            // SigningKeypair doesn't expose secret bytes directly — re-derive
            // the seed and use it as the secret half for Veilid's KeyPair format
            let mut ikm = Vec::with_capacity(36);
            ikm.extend_from_slice(&slot_seed);
            ikm.extend_from_slice(&approved.slot_index.to_le_bytes());
            let seed = blake3::derive_key(
                rekindle_identity::derivation_tags::SLOT_KEYPAIR,
                &ikm,
            );
            buf[32..].copy_from_slice(&seed);
            buf
        };

        for ch in &channels {
            if let Some(ref record_key) = ch.message_record_key {
                if record_key.is_empty() { continue; }
                // Open the shared SMPL channel record with our slot keypair
                if let Err(e) = self.io.open_record(record_key, Some(&slot_keypair_bytes)).await {
                    tracing::warn!(
                        channel = %ch.name,
                        record_key = &record_key[..12.min(record_key.len())],
                        error = %e,
                        "channel SMPL record open failed — will retry on first message send"
                    );
                }
                channel_record_keys.insert(ch.id.clone(), record_key.clone());
            }
        }
        tracing::info!(
            channels = channel_record_keys.len(),
            slot = approved.slot_index,
            "channel SMPL records opened with slot keypair"
        );

        // Write signed MemberSummary to our registry slot, replacing
        // the operator's unsigned placeholder from approval.
        let pseudonym_seed = self.io.pseudonym_seed(&submitted.governance_key)?;
        let pseudonym_kp = rekindle_identity::SigningKeypair::from_seed(&pseudonym_seed)
            .map_err(|e| ChatError::Internal(format!("pseudonym keypair: {e}")))?;
        let community_x25519_seed = rekindle_identity::x25519_seed_from(&pseudonym_seed);
        let our_x25519_dh = rekindle_identity::x25519_public_from_raw_seed(&community_x25519_seed)
            .map_err(|e| ChatError::Internal(format!("x25519: {e}")))?;

        let display_name_for_member = {
            let meta = self.session_meta.read();
            meta.identity.as_ref().map(|i| i.display_name.clone()).unwrap_or_default()
        };
        let mut my_member = rekindle_types::dht_types::MemberSummary {
            pseudonym_key: submitted.pseudonym_hex.clone(),
            display_name: display_name_for_member.clone(),
            role_ids: Vec::new(),
            joined_at: crate::time::timestamp_ms(),
            subkey_index: approved.slot_index,
            onboarding_complete: true,
            timeout_until: None,
            x25519_pub: Some(our_x25519_dh.to_hex()),
            profile_dht_key: {
                let meta = self.session_meta.read();
                meta.identity.as_ref().map(|i| i.profile_dht_key.clone())
            },
            channel_records: HashMap::new(),
            signature: Vec::new(),
        };
        let sig = pseudonym_kp.sign_raw(&my_member.signing_bytes());
        my_member.signature = sig.to_vec();
        let my_member_bytes = serde_json::to_vec(&my_member)
            .map_err(|e| ChatError::Serialization(format!("signed member: {e}")))?;
        self.io.open_and_write(
            &submitted.registry_key, approved.slot_index, &my_member_bytes,
            Some(&slot_keypair_bytes), crate::io::Confirm::Accepted,
        ).await?;
        tracing::info!(slot = approved.slot_index, "signed MemberSummary written to registry");

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
                channel_name_to_id,
                channel_slowmode,
                community_mailbox_key: submitted.community_mailbox_key.clone(),
                join_inbox_key: submitted.join_inbox_key.clone(),
                is_operator: false,
                locked_down: false,
                joined_at: timestamp_ms(),
                last_seen_seqs: HashMap::new(),
                slot_seed: Some(metadata.slot_seed_hex.clone()),
                lamport_counter: 0,
            });
        }

        // Discover routes for existing community members → populate gossip mesh
        self.discover_community_member_routes(
            &submitted.governance_key,
            &submitted.pseudonym_hex,
            &submitted.registry_key,
        ).await;

        tracing::info!(
            community = %submitted.community_name,
            slot = approved.slot_index,
            channels = channels_discovered,
            meks = meks_cached,
            mesh_peers = self.io.transport().gossip_mesh_peer_count(),
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

    /// Join a community — returns existing membership if already joined.
    pub async fn join_community(
        &self,
        governance_key: &str,
    ) -> Result<JoinCompleted, ChatError> {
        // Idempotent: if already a member, return existing membership.
        {
            let meta = self.session_meta.read();
            if let Some(m) = meta.communities.get(governance_key) {
                return Ok(JoinCompleted {
                    community_name: m.community_name.clone(),
                    governance_key: governance_key.to_string(),
                    pseudonym_hex: m.pseudonym_key.clone(),
                    slot_index: m.slot_index,
                    registry_key: m.registry_key.clone(),
                    community_mailbox_key: m.community_mailbox_key.clone(),
                    channels_discovered: m.channel_name_to_id.len(),
                    meks_cached: 0,
                });
            }
            // Also check by name in case governance key format differs
            if let Some(m) = meta.community_by_name(governance_key) {
                return Ok(JoinCompleted {
                    community_name: m.community_name.clone(),
                    governance_key: m.governance_key.clone(),
                    pseudonym_hex: m.pseudonym_key.clone(),
                    slot_index: m.slot_index,
                    registry_key: m.registry_key.clone(),
                    community_mailbox_key: m.community_mailbox_key.clone(),
                    channels_discovered: m.channel_name_to_id.len(),
                    meks_cached: 0,
                });
            }
        }

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
        let raw = self.read_verified_governance(governance_key, MANIFEST_METADATA).await?
            .ok_or_else(|| ChatError::CommunityNotFound { community: governance_key.into() })?;
        serde_json::from_slice(&raw)
            .map_err(|e| ChatError::Deserialization(format!("governance metadata: {e}")))
    }

    async fn read_governance_metadata_fresh(
        &self, governance_key: &str,
    ) -> Result<CommunityMetadata, ChatError> {
        let raw = self.read_verified_governance(governance_key, MANIFEST_METADATA).await?
            .ok_or_else(|| ChatError::CommunityNotFound { community: governance_key.into() })?;
        serde_json::from_slice(&raw)
            .map_err(|e| ChatError::Deserialization(format!("governance metadata: {e}")))
    }

    async fn read_registry_key(
        &self, governance_key: &str, community_name: &str,
    ) -> Result<String, ChatError> {
        let raw = self.read_verified_governance(governance_key, MANIFEST_REGISTRY_SPINE).await?
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

    /// Unwrap slot_seed from the MEK vault's wrapped_slot_seeds.
    async fn unwrap_slot_seed_from_vault(
        &self, governance_key: &str, our_pseudonym_hex: &str,
    ) -> Result<[u8; 32], ChatError> {
        let vault = self.read_mek_vault_from_metadata(governance_key).await?;
        let pseudonym_seed = self.io.pseudonym_seed(governance_key)?;
        let our_x25519_seed = rekindle_identity::x25519_seed_from(&pseudonym_seed);

        for entry in &vault {
            for ws in &entry.wrapped_slot_seeds {
                if ws.target_pseudonym != our_pseudonym_hex { continue; }
                let Some(ref rotator_dh) = entry.rotator_x25519_pub else { continue; };
                let decrypted = crate::crypto::mek::unwrap_mek(
                    &our_x25519_seed, rotator_dh, &ws.encrypted_slot_seed,
                )?;
                if decrypted.len() != 32 {
                    return Err(ChatError::Internal(format!("slot_seed wrong length: {}", decrypted.len())));
                }
                let mut seed = [0u8; 32];
                seed.copy_from_slice(&decrypted);
                return Ok(seed);
            }
        }
        Err(ChatError::Internal("no wrapped_slot_seed found for our pseudonym in vault".into()))
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
        let our_x25519_seed = rekindle_identity::x25519_seed_from(&pseudonym_seed);

        let mut cached = 0usize;
        for entry in &vault {
            let Some(copy) = entry.copies.iter().find(|c| c.target_pseudonym == our_pseudonym_hex) else {
                continue;
            };

            let Some(ref rotator_dh_key) = entry.rotator_x25519_pub else {
                tracing::error!(
                    channel = %entry.channel_id,
                    rotator = &entry.rotator_pseudonym[..12.min(entry.rotator_pseudonym.len())],
                    "vault entry missing rotator_x25519_pub — cannot unwrap MEK"
                );
                continue;
            };

            match crate::crypto::mek::unwrap_mek(&our_x25519_seed, rotator_dh_key, &copy.encrypted_mek) {
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
            let data = self.io.open_and_read(registry_key, REGISTRY_MEK_VAULT, true)
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
        if let Err(e) = self.io.open_and_watch(
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
        if let Err(e) = self.io.open_and_watch(
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
        if let Err(e) = self.io.open_and_watch(
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
