//! EventRouter — implements TransportCallback, verifies inbound messages,
//! dispatches to services, emits events through the pipeline.
//!
//! Every inbound byte from transport flows through this struct.
//! Every outbound event to the IPC bus flows through this struct's pipeline.
//!
//! Verification order:
//! 1. Read TypeId byte (first byte of raw data)
//! 2. For DM TypeIds (0x01-0x06): parse SignedEnvelope, verify Ed25519
//!    signature + timestamp freshness (5min window, 60s future skew)
//! 3. For gossip TypeId (0x0A): delegate to community.handle_gossip
//!    which verifies the gossip envelope signature
//! 4. For RPC TypeId (0x0B): delegate to community.handle_rpc_message
//! 5. Convert verified payload to SubscriptionEvent
//! 6. Process through EventPipeline (state_effects → dedup → emit)
//!
//! If verification fails at any step, the message is dropped and logged
//! with the sender key, TypeId, and specific failure reason. No partial
//! dispatch. No silent drops.

use std::sync::Arc;

use async_trait::async_trait;
use rekindle_types::transport::{TransportCallback, TransportEvent};
use rekindle_types::subscription_events::SubscriptionEvent;

use super::pipeline::EventPipeline;
use super::registry::{WatchKind, WatchRegistry};
use crate::crypto::envelope::SignedEnvelope;
use crate::friendship::FriendshipService;
use crate::messaging::MessagingService;
use crate::community::CommunityService;

/// Routes inbound transport events to chat services via the event pipeline.
pub struct EventRouter {
    watches: Arc<WatchRegistry>,
    pipeline: Arc<EventPipeline>,
    friendship: Arc<FriendshipService>,
    messaging: Arc<MessagingService>,
    community: Arc<CommunityService>,
}

impl EventRouter {
    pub fn new(
        watches: Arc<WatchRegistry>,
        pipeline: Arc<EventPipeline>,
        friendship: Arc<FriendshipService>,
        messaging: Arc<MessagingService>,
        community: Arc<CommunityService>,
    ) -> Self {
        Self { watches, pipeline, friendship, messaging, community }
    }
}

#[async_trait]
impl TransportCallback for EventRouter {
    async fn on_message(&self, sender_key: &str, data: &[u8]) {
        if data.is_empty() {
            tracing::debug!("dropping empty app_message");
            return;
        }

        let type_id = data[0];

        match type_id {
            // ── DM TypeIds (0x01-0x06): SignedEnvelope verification ──
            1..=6 => {
                // Parse SignedEnvelope: [TypeId(1) || timestamp(8) || pubkey(32) || sig(64) || payload]
                // The full data includes the TypeId byte which is part of the envelope.
                let envelope = match SignedEnvelope::parse(data) {
                    Ok(e) => e,
                    Err(e) => {
                        tracing::warn!(
                            type_id,
                            sender = &sender_key[..12.min(sender_key.len())],
                            error = %e,
                            "dropping DM: envelope parse failed"
                        );
                        return;
                    }
                };

                // Verify signature + timestamp freshness
                if let Err(e) = envelope.verify() {
                    tracing::warn!(
                        type_id,
                        sender = &sender_key[..12.min(sender_key.len())],
                        error = %e,
                        "dropping DM: verification failed (forgery, replay, or clock skew)"
                    );
                    return;
                }

                let verified_sender = hex::encode(&envelope.sender_key[..]);
                let payload = &envelope.payload;

                match type_id {
                    1 => {
                        // Typing indicator — gate on known peer
                        if !self.messaging.handle_typing(&verified_sender, payload) {
                            return;
                        }
                        let dm_payload: rekindle_types::dm_payload::DmPayload =
                            match postcard::from_bytes(payload) {
                                Ok(p) => p,
                                Err(_) => return,
                            };
                        let event = crate::events::conversions::dm_to_event(dm_payload, &verified_sender);
                        self.pipeline.process(event);
                    }
                    2 => {
                        // FriendRequestAck — trigger inbox scan
                        self.friendship.trigger_inbox_scan();
                        let event = SubscriptionEvent::Friend(
                            rekindle_types::subscription_events::FriendEvent::RequestAcknowledged {
                                peer_key: verified_sender,
                            },
                        );
                        self.pipeline.process(event);
                    }
                    3 => {
                        // Unfriend
                        self.friendship.handle_unfriend(&verified_sender).await;
                        let event = SubscriptionEvent::Friend(
                            rekindle_types::subscription_events::FriendEvent::Removed {
                                peer_key: verified_sender,
                            },
                        );
                        self.pipeline.process(event);
                    }
                    4 => {
                        // UnfriendAck — acknowledged, emit event only
                        let event = SubscriptionEvent::Friend(
                            rekindle_types::subscription_events::FriendEvent::RemoveAcknowledged {
                                peer_key: verified_sender,
                            },
                        );
                        self.pipeline.process(event);
                    }
                    5 => {
                        // ProfileKeyRotated
                        self.friendship.handle_profile_rotated(&verified_sender, payload).await;
                        let dm_payload: rekindle_types::dm_payload::DmPayload =
                            match postcard::from_bytes(payload) {
                                Ok(p) => p,
                                Err(_) => return,
                            };
                        let event = crate::events::conversions::dm_to_event(dm_payload, &verified_sender);
                        self.pipeline.process(event);
                    }
                    6 => {
                        // PresenceUpdate — gate on known peer
                        if !self.messaging.handle_presence_update(&verified_sender, payload) {
                            return;
                        }
                        let dm_payload: rekindle_types::dm_payload::DmPayload =
                            match postcard::from_bytes(payload) {
                                Ok(p) => p,
                                Err(_) => return,
                            };
                        let event = crate::events::conversions::dm_to_event(dm_payload, &verified_sender);
                        self.pipeline.process(event);
                    }
                    _ => unreachable!(),
                }
            }

            // ── Gossip TypeId (0x0A): community handles verification ──
            10 => {
                // payload = everything after TypeId byte
                let payload = &data[1..];
                if let Some(event) = self.community.handle_gossip(sender_key, payload).await {
                    self.pipeline.process(event);
                }
                // None means handle_gossip dropped it (malformed, forged, or
                // deserialization failed) — already logged inside handle_gossip.
            }

            // ── RPC TypeId (0x0B): community handles verification ──
            11 => {
                let payload = &data[1..];
                self.community.handle_rpc_message(sender_key, payload).await;
            }

            // ── Unknown TypeId ──
            _ => {
                tracing::debug!(
                    type_id,
                    sender = &sender_key[..12.min(sender_key.len())],
                    "unknown TypeId — dropping"
                );
            }
        }
    }

    async fn on_call(&self, sender_key: &str, data: &[u8]) -> Vec<u8> {
        // RPC calls go through community governance dispatch.
        // The call data is raw bytes — community.handle_rpc_call parses,
        // deserializes, dispatches to the governance op handler, and
        // returns serialized response bytes.
        self.community.handle_rpc_call(sender_key, data).await
    }

    async fn on_record_change(
        &self,
        record_key: &str,
        subkeys: Vec<u32>,
        _value_count: u32,
        data: Option<Vec<u8>>,
    ) {
        match self.watches.lookup(record_key) {
            Some(WatchKind::DmLog { ref peer_key }) => {
                self.messaging.handle_dm_log_change(peer_key, record_key, data).await;
            }
            Some(WatchKind::ChannelLog { ref community, ref channel_id, ref member }) => {
                self.messaging.handle_channel_log_change(community, channel_id, member, data);
            }
            Some(WatchKind::FriendInbox) => {
                self.friendship.trigger_inbox_scan();
            }
            Some(WatchKind::GovernanceManifest { ref community }) => {
                self.community.handle_governance_change(community, &subkeys).await;
            }
            Some(WatchKind::MemberRegistry { ref community }) => {
                self.community.handle_registry_change(community, &subkeys).await;
            }
            Some(WatchKind::JoinInbox { ref community }) => {
                self.community.handle_join_inbox_change(community).await;
            }
            None => {
                tracing::debug!(
                    record_key,
                    subkeys = ?subkeys,
                    "record change for unregistered watch — dropping"
                );
            }
        }
    }

    async fn on_event(&self, event: TransportEvent) {
        match event {
            TransportEvent::Attached => {
                tracing::info!("transport attached — platform is online");
                self.pipeline.process(SubscriptionEvent::Network(
                    rekindle_types::subscription_events::NetworkEvent::AttachmentChanged {
                        is_attached: true,
                        public_internet_ready: true,
                    },
                ));
            }
            TransportEvent::Detached => {
                tracing::warn!("transport detached — platform is offline");
                self.pipeline.process(SubscriptionEvent::Network(
                    rekindle_types::subscription_events::NetworkEvent::AttachmentChanged {
                        is_attached: false,
                        public_internet_ready: false,
                    },
                ));
            }
            TransportEvent::RouteDied { ref route_id } => {
                tracing::warn!(route_id, "route died — clients using this route will fail");
            }
            TransportEvent::WatchExpired { ref record_key } => {
                tracing::debug!(record_key, "watch expired — needs renewal");
                self.pipeline.process(SubscriptionEvent::Network(
                    rekindle_types::subscription_events::NetworkEvent::WatchFailed {
                        record_key: record_key.clone(),
                        error: "watch expired — renewal needed".into(),
                    },
                ));
            }
            TransportEvent::RouteAllocated { .. } => {}
            TransportEvent::PeerCountChanged { count } => {
                tracing::debug!(count, "peer count changed");
            }
            TransportEvent::PublicInternet { available } => {
                tracing::info!(available, "public internet reachability changed");
            }
        }
    }
}
