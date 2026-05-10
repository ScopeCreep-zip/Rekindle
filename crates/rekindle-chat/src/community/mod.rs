//! Community operations — create, join, leave, governance, membership,
//! inbound gossip/RPC dispatch, DHT watch handlers.

pub mod create;
pub mod join;
pub mod leave;
pub mod governance;
pub mod membership;
pub mod social;
pub mod system;

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
    pub async fn handle_gossip(
        &self,
        sender_key: &str,
        payload: &[u8],
    ) -> Option<SubscriptionEvent> {
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

        // Verify Ed25519 signature over payload_bytes
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

        if rekindle_ratchet::crypto::sign::verify_ec_prekey(
            &pub_bytes, &envelope.payload_bytes, &sig_bytes,
        ).is_err() {
            tracing::warn!(
                sender = &sender_key[..12.min(sender_key.len())],
                community = &envelope.community_id[..12.min(envelope.community_id.len())],
                "gossip: SIGNATURE VERIFICATION FAILED — dropping (possible forgery)"
            );
            return None;
        }

        // Deserialize inner payload
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

        // Handle MEK-specific gossip payloads that need action beyond event emission
        if let GossipPayload::Control(ref ctrl) = gossip_payload {
            match ctrl {
                ControlPayload::RequestMek { channel_id, needed_generation, requester_pseudonym } => {
                    if let Err(e) = self.handle_mek_request(
                        &envelope.community_id, channel_id, requester_pseudonym, *needed_generation,
                    ).await {
                        tracing::debug!(error = %e, "MEK request handling failed (may not be operator)");
                    }
                }
                ControlPayload::MekTransfer { community_id, channel_id, generation, sender_pseudonym, wrapped_mek } => {
                    let ch = channel_id.as_deref().unwrap_or("unknown");
                    if let Err(e) = self.receive_mek_transfer(
                        community_id, ch, *generation, sender_pseudonym, wrapped_mek,
                    ) {
                        tracing::warn!(error = %e, "MEK transfer receive failed");
                    }
                }
                ControlPayload::ChannelLockdown { locked } => {
                    // Update cached lockdown state — the messaging send path
                    // reads this to enforce lockdown without DHT reads per message.
                    let mut meta = self.session_meta.write();
                    if let Some(membership) = meta.communities.get_mut(&envelope.community_id) {
                        membership.locked_down = *locked;
                        tracing::info!(
                            community = &envelope.community_id[..12.min(envelope.community_id.len())],
                            locked,
                            "lockdown state updated from gossip"
                        );
                    }
                }
                _ => {}
            }
        }

        // Convert to SubscriptionEvent for the dedup + state_effects + IPC pipeline
        let event = crate::events::conversions::gossip_to_event(
            gossip_payload,
            &envelope.community_id,
            &envelope.sender_pseudonym,
        );

        tracing::debug!(
            community = &envelope.community_id[..12.min(envelope.community_id.len())],
            sender = &envelope.sender_pseudonym[..12.min(envelope.sender_pseudonym.len())],
            "gossip: verified and dispatched"
        );

        Some(event)
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

    pub async fn handle_rpc_call(&self, sender_key: &str, data: &[u8]) -> Vec<u8> {
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
            InboundCall::Dm(_data) => {
                tracing::debug!("DM via RPC received — forwarding to messaging");
                CallResponse::Ack
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
            match self.io.read_record(community, subkey, true).await {
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
        let raw = self.io
            .read_record(governance_key, rekindle_types::dht_types::MANIFEST_METADATA, true)
            .await?
            .ok_or_else(|| ChatError::CommunityNotFound {
                community: governance_key.into(),
            })?;
        serde_json::from_slice(&raw)
            .map_err(|e| ChatError::Deserialization(format!("community metadata: {e}")))
    }
}
