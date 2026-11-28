//! The daemon's community gossip ingest (`InboundHandler::on_gossip`): the
//! own-removal and departed-author gate, then each control payload to its
//! Host owner or the `SubscriptionManager`; and the gate the handler's
//! other gossip paths share.

use rekindle_codec::community::envelope::{CommunityEnvelope, ControlPayload};
use tracing::debug;

use super::DaemonHandler;

impl DaemonHandler {
    /// Ingest one verified gossip envelope from `sender_pseudonym`.
    pub(super) async fn ingest_gossip(
        &self,
        community_id: &str,
        sender_pseudonym: &str,
        envelope: CommunityEnvelope,
        lamport_ts: u64,
    ) {
        debug!(community = community_id, sender = %sender_pseudonym, "handler: on_gossip");
        if !self.takes_traffic(community_id, sender_pseudonym) {
            return;
        }

        // Mutual Aid §14.3 — plumbing, not news. The Host's one ingest (plan
        // GI3, the desktop's relay path calls it too) reads the record and
        // checks the hash; never surfaced as a subscription event.
        if let CommunityEnvelope::WatchRelay {
            ref record_key,
            subkey,
            ref content_hash,
            ref observer_pseudonym,
        } = envelope
        {
            if let Some(ctx) = self.host.get().and_then(std::sync::Weak::upgrade) {
                crate::host::adapters::governance::HostGovernance::new(ctx)
                    .on_watch_relay(record_key, subkey, content_hash, observer_pseudonym)
                    .await;
            }
            return;
        }

        // A joiner's Plate Gate request (plan GI3): the Host expands when we
        // hold MANAGE_COMMUNITY, the same on the desktop's relay path.
        if let Some(payload @ ControlPayload::RequestSegmentExpansion { .. }) =
            envelope.as_control()
        {
            if let Some(ctx) = self.host.get().and_then(std::sync::Weak::upgrade) {
                crate::host::adapters::governance::HostGovernance::new(ctx)
                    .on_segment_expansion_request(community_id, payload)
                    .await;
            }
        }

        // Channel typing (plan E3.4 S9) is the Host's: its one ingest gates,
        // meters, names the channel behind the tag and lapses the start, the
        // same on the desktop's relay path.
        if let CommunityEnvelope::TypingIndicator {
            channel_tag,
            ttl_ms,
            ref pseudonym_key,
        } = envelope
        {
            if let Some(ctx) = self.host.get().and_then(std::sync::Weak::upgrade) {
                crate::host::adapters::governance::HostGovernance::new(ctx).ingest_typing(
                    community_id,
                    sender_pseudonym,
                    lamport_ts,
                    pseudonym_key,
                    &channel_tag,
                    ttl_ms,
                );
            }
            return;
        }

        // A reaction (plan GI3) is the Host's: its one ingest gates, meters
        // and journals the event, the same on the desktop's relay path.
        if let Some(
            payload @ (ControlPayload::ReactionAdded { .. }
            | ControlPayload::ReactionRemoved { .. }),
        ) = envelope.as_control()
        {
            if let Some(ctx) = self.host.get().and_then(std::sync::Weak::upgrade) {
                crate::host::adapters::governance::HostGovernance::new(ctx).ingest_reaction(
                    community_id,
                    sender_pseudonym,
                    lamport_ts,
                    payload,
                );
            }
            return;
        }

        // The membership control messages (plan E3.1 B2e): onboarding
        // answers, a member's own departure and a relayed kick (GI3: the
        // Host checks the kicker's authority, then forgets and rotates).
        if let Some(payload) = envelope.as_control() {
            if let Some(ctx) = self.host.get().and_then(std::sync::Weak::upgrade) {
                crate::host::adapters::governance::HostGovernance::new(ctx)
                    .on_membership_control(community_id, sender_pseudonym, payload)
                    .await;
            }
        }

        // A joiner's (or a member's missed) key request (plan E3.2 S6): the
        // Host's responder answers members only.
        if let Some(ControlPayload::RequestMEK {
            channel_id,
            needed_generation,
            requester_pseudonym,
            cascade_index,
        }) = envelope.as_control()
        {
            if let Some(ctx) = self.host.get().and_then(std::sync::Weak::upgrade) {
                let request = crate::host::adapters::governance::MekRequest {
                    channel_id: channel_id.clone(),
                    needed_generation: *needed_generation,
                    requester_pseudonym: requester_pseudonym.clone(),
                    cascade_index: *cascade_index,
                };
                let deps =
                    crate::daemon::mek_rotation::DaemonMekAdapter::new(std::sync::Arc::clone(&ctx));
                if let Err(error) = crate::host::adapters::governance::HostGovernance::new(ctx)
                    .answer_mek_request(&deps, community_id, &request)
                    .await
                {
                    debug!(community = community_id, %error, "RequestMEK not answered");
                }
            }
        }

        // A signed governance op (plan E3.2, `governanceOp @74`).
        if let Some(ControlPayload::GovernanceOp { op }) = envelope.as_control() {
            if let Some(ctx) = self.host.get().and_then(std::sync::Weak::upgrade) {
                let host = crate::host::adapters::governance::HostGovernance::new(ctx);
                host.on_governance_op(community_id, op.clone()).await;
            }
            return;
        }

        // A game server added or removed (plan E3.1 H1): stored when the
        // Host's adapter accepts it.
        if let Some(
            payload @ (ControlPayload::GameServerAdded { .. }
            | ControlPayload::GameServerRemoved { .. }),
        ) = envelope.as_control()
        {
            if let Some(ctx) = self.host.get().and_then(std::sync::Weak::upgrade) {
                crate::host::adapters::governance::HostGovernance::new(ctx)
                    .on_game_server_control(community_id, sender_pseudonym, payload)
                    .await;
            }
        }

        // A message pinned or unpinned (plan E3.1 E3b): stored when the
        // Host's adapter accepts it.
        if let Some(
            payload @ (ControlPayload::MessagePinned { .. }
            | ControlPayload::MessageUnpinned { .. }),
        ) = envelope.as_control()
        {
            if let Some(ctx) = self.host.get().and_then(std::sync::Weak::upgrade) {
                crate::host::adapters::governance::HostGovernance::new(ctx)
                    .on_pin_control(community_id, sender_pseudonym, payload)
                    .await;
            }
        }

        // A thread reply (plan E3.1 E3f): stored by the Host's channel
        // adapter, which returns the event to publish carrying the body it
        // read from the thread record (the transport maps no thread
        // payload). A thread's opening and archive are signed governance
        // ops, never a gossip frame (plan E3.2 S8-E).
        if let Some(payload @ ControlPayload::ThreadMessageReceived { .. }) = envelope.as_control()
        {
            if let Some(ctx) = self.host.get().and_then(std::sync::Weak::upgrade) {
                let channel = crate::host::adapters::channel::HostChannel::new(&ctx);
                let scope = rekindle_governance_runtime::deps::GovernanceRuntimeDeps::scope(
                    &crate::daemon::dispatch::adapter(&ctx),
                );
                let (community, sender, payload, task_scope) = (
                    community_id.to_string(),
                    sender_pseudonym.to_string(),
                    payload.clone(),
                    scope.clone(),
                );
                scope.spawn_or_drop("thread control", async move {
                    if let Some(event) = channel
                        .on_thread_control(&community, &sender, &payload, &task_scope)
                        .await
                    {
                        ctx.publish_event(
                            rekindle_types::subscription_events::SubscriptionEvent::Social(event),
                        );
                    }
                });
            }
        }

        // An event created, updated, deleted or answered (plan E3.1 E4):
        // stored when the Host's adapter accepts it.
        if let Some(
            payload @ (ControlPayload::EventCreated { .. }
            | ControlPayload::EventUpdated { .. }
            | ControlPayload::EventDeleted { .. }
            | ControlPayload::EventRsvpChanged { .. }),
        ) = envelope.as_control()
        {
            if let Some(ctx) = self.host.get().and_then(std::sync::Weak::upgrade) {
                crate::host::adapters::governance::HostGovernance::new(ctx)
                    .on_event_control(community_id, sender_pseudonym, payload)
                    .await;
            }
        }

        if let Some(payload) = envelope.as_control() {
            if rekindle_voice::signaling::is_voice_signaling(payload) {
                if let Some(ctx) = self.host.get().and_then(std::sync::Weak::upgrade) {
                    if self
                        .subscriptions
                        .read()
                        .as_ref()
                        .is_some_and(|sub| sub.admit(community_id, sender_pseudonym, lamport_ts))
                    {
                        crate::host::adapters::voice::handle_signaling(
                            &ctx,
                            community_id,
                            sender_pseudonym,
                            payload.clone(),
                        );
                    }
                }
                return;
            }
        }
        if let Some(ref sub_mgr) = *self.subscriptions.read() {
            sub_mgr.on_gossip(community_id, sender_pseudonym, envelope, lamport_ts);
        }
    }

    /// Whether we take `author`'s gossip for the community
    /// (`CommunityStore::takes_traffic_from`).
    pub(super) fn takes_traffic(&self, community_id: &str, author: &str) -> bool {
        let author = rekindle_types::id::PseudonymKey::from_hex_lossy(author);
        (self.host.get().and_then(std::sync::Weak::upgrade))
            .is_none_or(|ctx| ctx.communities.takes_traffic_from(community_id, &author))
    }

    /// Whether we relay `author`'s gossip for the community one hop
    /// further (`CommunityStore::relays_for`): never for a community the
    /// Host does not hold, nor before it is up.
    pub(super) fn relays(&self, community_id: &str, author: &str) -> bool {
        let author = rekindle_types::id::PseudonymKey::from_hex_lossy(author);
        (self.host.get().and_then(std::sync::Weak::upgrade))
            .is_some_and(|ctx| ctx.communities.relays_for(community_id, &author))
    }
}
