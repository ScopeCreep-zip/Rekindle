//! Community operations — create, join, leave, governance, membership,
//! inbound gossip/RPC dispatch, DHT watch handlers.

pub mod create;
pub mod join;
pub mod leave;
pub mod governance;
pub mod membership;
pub mod social;
pub mod system;
mod gossip_dispatch;

use std::sync::Arc;

use parking_lot::RwLock;
use rekindle_storage::VaultStore;
use rekindle_types::gossip_payload::{GossipPayload, ControlPayload};
use rekindle_types::rpc_payload::{GovernanceRequest, InboundCall, CallResponse};
use rekindle_types::session_types::SessionMeta;
use rekindle_types::subscription_events::SubscriptionEvent;

use crate::crypto::mek::MekCache;
use crate::events::pipeline::EventPipeline;
use crate::events::registry::WatchRegistry;
use crate::io::PlatformIO;
use crate::ChatError;

pub struct CommunityService {
    pub(crate) io: Arc<PlatformIO>,
    pub(crate) vault: Arc<VaultStore>,
    pub(crate) session_meta: Arc<RwLock<SessionMeta>>,
    pub(crate) mek_cache: Arc<MekCache>,
    pub(crate) watches: Arc<WatchRegistry>,
    pub(crate) pipeline: Arc<EventPipeline>,
}

impl CommunityService {
    // ── Inbound gossip dispatch ─────────────────────────────────

    /// Handle an inbound gossip message. Verifies signature, deserializes
    /// payload, dispatches to the appropriate handler.
    ///
    /// Returns `Some(SubscriptionEvent)` if the gossip produced an event
    /// that should be emitted through the dedup + state_effects + IPC pipeline.
    /// Returns `None` if the gossip was malformed, forged, or produced no event.
    ///
    /// Errors are logged with full context (sender, community, error detail)
    /// and return None — a malformed gossip message from one peer must not
    /// crash the daemon or block processing of subsequent messages.
    /// Handle an inbound gossip message.
    ///
    /// Verification skeleton: deserialize envelope, verify Ed25519 signature,
    /// deserialize inner payload. Then delegates to `gossip_dispatch` for
    /// route extraction, TTL forwarding, MEK decrypt, and event conversion.
    ///
    /// `RequestMek` is handled here (not in gossip_dispatch) because it
    /// calls `self.handle_mek_request()` which requires `&CommunityService`.
    pub async fn handle_gossip(
        &self,
        sender_key: &str,
        payload: &[u8],
    ) -> Option<SubscriptionEvent> {
        tracing::info!(
            payload_len = payload.len(),
            sender = if sender_key.is_empty() { "anonymous" } else { &sender_key[..16.min(sender_key.len())] },
            "handle_gossip: ENTERED — deserializing envelope"
        );

        let envelope: rekindle_types::gossip_payload::SignedGossipEnvelope =
            match postcard::from_bytes(payload) {
                Ok(e) => e,
                Err(e) => {
                    tracing::warn!(
                        sender = &sender_key[..12.min(sender_key.len())],
                        error = %e,
                        "gossip: envelope deserialization failed — dropping"
                    );
                    return None;
                }
            };

        // ── Verify Ed25519 signature ───────────────────────────────
        let Some(pub_bytes) = hex::decode(&envelope.sender_pseudonym)
            .ok()
            .and_then(|b| <[u8; 32]>::try_from(b.as_slice()).ok()) else {
            tracing::warn!(
                sender = &sender_key[..12.min(sender_key.len())],
                pseudonym = &envelope.sender_pseudonym[..12.min(envelope.sender_pseudonym.len())],
                "gossip: invalid sender pseudonym hex — dropping"
            );
            return None;
        };

        let Ok(sig_bytes): Result<[u8; 64], _> = envelope.signature.as_slice().try_into() else {
            tracing::warn!(
                sender = &sender_key[..12.min(sender_key.len())],
                sig_len = envelope.signature.len(),
                "gossip: signature wrong length (expected 64) — dropping"
            );
            return None;
        };

        if rekindle_identity::verify_ec_prekey(
            &pub_bytes, &envelope.payload_bytes, &sig_bytes,
        ).is_err() {
            tracing::warn!(
                sender = &sender_key[..12.min(sender_key.len())],
                community = &envelope.community_id[..12.min(envelope.community_id.len())],
                "gossip: SIGNATURE VERIFICATION FAILED — dropping (possible forgery)"
            );
            return None;
        }

        tracing::info!(
            community = &envelope.community_id[..20.min(envelope.community_id.len())],
            sender_pseudonym = &envelope.sender_pseudonym[..16.min(envelope.sender_pseudonym.len())],
            ttl = envelope.ttl,
            payload_bytes_len = envelope.payload_bytes.len(),
            "handle_gossip: signature VERIFIED — deserializing inner payload"
        );

        let gossip_payload: GossipPayload = match postcard::from_bytes(&envelope.payload_bytes) {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(
                    sender = &sender_key[..12.min(sender_key.len())],
                    community = &envelope.community_id[..12.min(envelope.community_id.len())],
                    error = %e,
                    "gossip: inner payload deserialization failed — dropping"
                );
                return None;
            }
        };

        match &gossip_payload {
            GossipPayload::Control(ControlPayload::RequestMek {
                ref channel_id, needed_generation, ref requester_pseudonym,
            }) => {
                if let Err(e) = self.handle_mek_request(
                    &envelope.community_id, channel_id, requester_pseudonym, *needed_generation,
                ).await {
                    tracing::debug!(error = %e, "MEK request handling failed (may not be operator)");
                }
            }
            GossipPayload::Control(ControlPayload::MekTransfer {
                ref community_id, ref channel_id, generation, ref sender_pseudonym,
                ref wrapped_mek, ref sender_x25519_pub,
            }) => {
                let ch = channel_id.as_deref().unwrap_or("unknown");
                let Some(ref sender_dh_key) = sender_x25519_pub else {
                    tracing::warn!(
                        sender = &sender_pseudonym[..12.min(sender_pseudonym.len())],
                        "MekTransfer missing sender_x25519_pub — dropping"
                    );
                    return None;
                };
                if let Err(e) = self.receive_mek_transfer(
                    community_id, ch, *generation, wrapped_mek, sender_dh_key,
                ) {
                    tracing::warn!(error = %e, "MEK transfer receive failed");
                }
            }
            _ => {}
        }

        // ── Delegate to gossip_dispatch for everything else ────────
        gossip_dispatch::dispatch_verified_gossip(
            &self.io, &self.vault, &self.mek_cache, &self.session_meta,
            &envelope, gossip_payload,
        ).await
    }

    // ── Inbound RPC dispatch ────────────────────────────────────

    pub async fn handle_rpc_message(&self, sender_key: &str, payload: &[u8]) {
        let request: GovernanceRequest = match postcard::from_bytes(payload) {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(
                    sender = &sender_key[..12.min(sender_key.len())],
                    error = %e,
                    "RPC: governance request deserialization failed"
                );
                return;
            }
        };

        if let Err(e) = self.handle_governance_op(sender_key, request).await {
            tracing::warn!(
                sender = &sender_key[..12.min(sender_key.len())],
                error = %e,
                "RPC: governance operation failed"
            );
        }
    }

    pub async fn handle_rpc_call(&self, sender_key: &str, data: &[u8], dm_deps: &dyn crate::dm::DmDeps) -> Vec<u8> {
        let call: InboundCall = match postcard::from_bytes(data) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(
                    sender = &sender_key[..12.min(sender_key.len())],
                    error = %e,
                    "RPC call: deserialization failed"
                );
                let resp = CallResponse::Rejected { reason: format!("deserialize: {e}") };
                return postcard::to_stdvec(&resp).unwrap_or_else(|_| b"NAK".to_vec());
            }
        };

        let response = match call {
            InboundCall::CommunityGovOp(req) => {
                match self.handle_governance_op(sender_key, req).await {
                    Ok(()) => CallResponse::Ack,
                    Err(e) => CallResponse::Rejected { reason: format!("{e}") },
                }
            }
            InboundCall::CommunityLeave(leave) => {
                if let Err(e) = self.handle_member_leave(
                    &leave.governance_key, &leave.leaving_pseudonym_hex,
                ).await {
                    tracing::warn!(error = %e, "leave notification handling failed");
                }
                CallResponse::Ack
            }
            InboundCall::Sync(_req) => {
                tracing::debug!("sync request received — not yet implemented");
                CallResponse::Ack
            }
            InboundCall::Dm(dm_data) => {
                match serde_json::from_slice::<crate::dm::invite::DmInvite>(&dm_data) {
                    Ok(invite) => {
                        tracing::info!(
                            record_key = &invite.record_key[..20.min(invite.record_key.len())],
                            sender = &sender_key[..16.min(sender_key.len())],
                            "community::rpc: DM invite via InboundCall::Dm"
                        );
                        match crate::dm::ingest::handle_incoming_dm_invite(
                            dm_deps, sender_key, &invite,
                        ).await {
                            Ok(()) => CallResponse::Ack,
                            Err(e) => {
                                tracing::warn!(error = %e, "community::rpc: DM invite ingest failed");
                                CallResponse::Rejected { reason: format!("dm invite: {e}") }
                            }
                        }
                    }
                    Err(e) => {
                        tracing::debug!(
                            error = %e,
                            "community::rpc: InboundCall::Dm payload is not a DmInvite"
                        );
                        CallResponse::Rejected { reason: format!("unrecognized DM payload: {e}") }
                    }
                }
            }
        };

        postcard::to_stdvec(&response).unwrap_or_else(|_| b"NAK".to_vec())
    }

    // ── DHT watch handlers ──────────────────────────────────────

    pub async fn handle_governance_change(&self, community: &str, subkeys: &[u32]) {
        tracing::info!(
            community = &community[..12.min(community.len())],
            subkeys = ?subkeys,
            "governance manifest changed — refreshing local state"
        );
        for &subkey in subkeys {
            match self.io.open_and_read(community, subkey, true).await {
                Ok(Some(_data)) => {
                    tracing::debug!(
                        community = &community[..12.min(community.len())],
                        subkey,
                        "governance subkey refreshed"
                    );
                }
                Ok(None) => {
                    tracing::debug!(
                        community = &community[..12.min(community.len())],
                        subkey,
                        "governance subkey empty"
                    );
                }
                Err(e) => {
                    tracing::error!(
                        community = &community[..12.min(community.len())],
                        subkey,
                        error = %e,
                        "governance subkey read FAILED — local state may be stale. \
                         Clients may show outdated channels, roles, or bans until \
                         the next successful poll sweep."
                    );
                }
            }
        }
    }

    pub async fn handle_registry_change(&self, community: &str, subkeys: &[u32]) {
        tracing::info!(
            community = &community[..12.min(community.len())],
            subkeys = ?subkeys,
            "member registry changed — refreshing member list"
        );
        let membership = {
            let meta = self.session_meta.read();
            meta.communities.get(community).cloned()
        };
        let Some(membership) = membership else {
            tracing::debug!(
                community = &community[..12.min(community.len())],
                "registry change for unknown community — ignoring"
            );
            return;
        };
        if let Err(e) = self.read_members(&membership.registry_key).await {
            tracing::error!(
                community = &community[..12.min(community.len())],
                error = %e,
                "member index read FAILED after registry change — \
                 member list may be stale until next poll sweep"
            );
        }
    }

    pub async fn handle_join_inbox_change(&self, community: &str) {
        tracing::info!(
            community = &community[..12.min(community.len())],
            "join inbox changed — processing pending requests"
        );
        match self.process_join_inbox(community).await {
            Ok(count) if count > 0 => {
                tracing::info!(
                    community = &community[..12.min(community.len())],
                    new = count,
                    "join inbox processed"
                );
            }
            Ok(_) => {}
            Err(e) => {
                tracing::error!(
                    community = &community[..12.min(community.len())],
                    error = %e,
                    "join inbox processing FAILED — pending requests may be \
                     missed until next inbox scan cycle"
                );
            }
        }
    }

    // ── Internal helpers ────────────────────────────────────────

    pub(crate) async fn read_metadata(
        &self,
        governance_key: &str,
    ) -> Result<rekindle_types::dht_types::CommunityMetadata, ChatError> {
        let raw = self
            .read_verified_governance(governance_key, rekindle_types::dht_types::MANIFEST_METADATA)
            .await?
            .ok_or_else(|| ChatError::CommunityNotFound {
                community: governance_key.into(),
            })?;
        serde_json::from_slice(&raw)
            .map_err(|e| ChatError::Deserialization(format!("community metadata: {e}")))
    }

    /// Discover routes for all members in a community and populate the gossip mesh.
    ///
    /// Reads the member registry from DHT, iterates members with profile_dht_key,
    /// reads each member's route blob from their profile, caches the route in the
    /// peer registry, and upserts into the community's gossip mesh.
    ///
    /// Called during resume and after join completion. Non-fatal — errors on
    /// individual members are logged and skipped.
    pub async fn discover_community_member_routes(
        &self,
        governance_key: &str,
        my_pseudonym: &str,
        registry_key: &str,
    ) {
        if registry_key.is_empty() {
            return;
        }
        let members = match self.list_members(governance_key).await {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!(
                    community = &governance_key[..20.min(governance_key.len())],
                    error = %e,
                    "member list read failed — gossip mesh will be empty"
                );
                return;
            }
        };

        let mut discovered = 0u32;
        for member in &members {
            // Skip ourselves
            if member.pseudonym_key == my_pseudonym {
                continue;
            }
            // Skip members without profile DHT keys
            let Some(ref profile_key) = member.profile_dht_key else {
                continue;
            };
            if profile_key.is_empty() {
                continue;
            }

            // Open the member's profile read-only
            if let Err(e) = self.io.open_record(profile_key, None).await {
                tracing::debug!(
                    member = &member.pseudonym_key[..16.min(member.pseudonym_key.len())],
                    error = %e,
                    "member profile open failed — skipping"
                );
                continue;
            }

            // Discover route and upsert into mesh
            let mesh_info = crate::io::route::MeshPeerInfo {
                community_id: governance_key,
                pseudonym: &member.pseudonym_key,
                status: "online",
                my_pseudonym,
            };
            match self.io.discover_peer_route(
                &member.pseudonym_key,
                profile_key,
                Some(mesh_info),
            ).await {
                Ok(true) => { discovered += 1; }
                Ok(false) => {}
                Err(e) => {
                    tracing::debug!(
                        member = &member.pseudonym_key[..16.min(member.pseudonym_key.len())],
                        error = %e,
                        "member route discovery failed — skipping"
                    );
                }
            }
        }

        tracing::info!(
            community = &governance_key[..20.min(governance_key.len())],
            total_members = members.len(),
            routes_discovered = discovered,
            "community member routes discovered"
        );
    }
}
