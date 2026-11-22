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

        // Mutual Aid §14.3 — plumbing, not news. Acted on by fetching
        // the record, never surfaced as a subscription event, so it is
        // intercepted before the pipeline.
        if let CommunityEnvelope::WatchRelay {
            ref record_key,
            subkey,
            ref content_hash,
            ref observer_pseudonym,
        } = envelope
        {
            self.on_watch_relay(record_key, subkey, content_hash, observer_pseudonym)
                .await;
            return;
        }

        // Channel typing (plan E3.4 S9) is the Host's: it names the channel
        // behind the tag and lapses the start. Metered like all gossip; a
        // frame naming someone other than its signer is dropped.
        if let CommunityEnvelope::TypingIndicator {
            channel_tag,
            ttl_ms,
            ref pseudonym_key,
        } = envelope
        {
            let admitted = (self.subscriptions.read().as_ref())
                .is_none_or(|sub| sub.admit(community_id, sender_pseudonym, lamport_ts));
            if admitted && pseudonym_key == sender_pseudonym {
                if let Some(ctx) = self.host.get().and_then(std::sync::Weak::upgrade) {
                    crate::host::adapters::governance::HostGovernance::new(ctx).on_channel_typing(
                        community_id,
                        sender_pseudonym,
                        &channel_tag,
                        ttl_ms,
                    );
                }
            }
            return;
        }

        // A kick rotates the keys the kicked member holds (plan D20). The
        // worker checks the sender's authority before acting.
        if let CommunityEnvelope::Control(ControlPayload::Kick {
            ref target_pseudonym,
        }) = envelope
        {
            let request = crate::daemon::mek_rotation::MekRotationRequest::kick(
                community_id,
                target_pseudonym.clone(),
                sender_pseudonym,
            );
            if self.mek_rotation_tx.send(request).is_err() {
                debug!(
                    community = community_id,
                    "MEK rotation worker stopped — kick not rotated"
                );
            }
        }

        // The membership control messages (plan E3.1 B2e): onboarding
        // answers and a member's own departure.
        if let CommunityEnvelope::Control(ref payload) = envelope {
            if let Some(ctx) = self.host.get().and_then(std::sync::Weak::upgrade) {
                crate::host::adapters::governance::HostGovernance::new(ctx)
                    .on_membership_control(community_id, sender_pseudonym, payload)
                    .await;
            }
        }

        // A joiner's (or a member's missed) key request (plan E3.2 S6): the
        // Host's responder answers members only.
        if let CommunityEnvelope::Control(ControlPayload::RequestMEK {
            ref channel_id,
            needed_generation,
            ref requester_pseudonym,
            cascade_index,
        }) = envelope
        {
            if let Some(ctx) = self.host.get().and_then(std::sync::Weak::upgrade) {
                let request = crate::host::adapters::governance::MekRequest {
                    channel_id: channel_id.clone(),
                    needed_generation,
                    requester_pseudonym: requester_pseudonym.clone(),
                    cascade_index,
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
        if let CommunityEnvelope::Control(ControlPayload::GovernanceOp { ref op }) = envelope {
            if let Some(ctx) = self.host.get().and_then(std::sync::Weak::upgrade) {
                let host = crate::host::adapters::governance::HostGovernance::new(ctx);
                host.on_governance_op(community_id, op.clone()).await;
            }
            return;
        }

        // A game server added or removed (plan E3.1 H1): stored when the
        // Host's adapter accepts it.
        if let CommunityEnvelope::Control(
            ref payload @ (ControlPayload::GameServerAdded { .. }
            | ControlPayload::GameServerRemoved { .. }),
        ) = envelope
        {
            if let Some(ctx) = self.host.get().and_then(std::sync::Weak::upgrade) {
                crate::host::adapters::governance::HostGovernance::new(ctx)
                    .on_game_server_control(community_id, sender_pseudonym, payload)
                    .await;
            }
        }

        // A message pinned or unpinned (plan E3.1 E3b): stored when the
        // Host's adapter accepts it.
        if let CommunityEnvelope::Control(
            ref payload @ (ControlPayload::MessagePinned { .. }
            | ControlPayload::MessageUnpinned { .. }),
        ) = envelope
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
        if let CommunityEnvelope::Control(
            ref payload @ ControlPayload::ThreadMessageReceived { .. },
        ) = envelope
        {
            if let Some(ctx) = self.host.get().and_then(std::sync::Weak::upgrade) {
                let channel = crate::host::adapters::channel::HostChannel::new(&ctx);
                let scope = rekindle_governance_runtime::deps::GovernanceRuntimeDeps::scope(
                    &crate::daemon::dispatch::adapter(&ctx),
                );
                let (community, payload, task_scope) =
                    (community_id.to_string(), payload.clone(), scope.clone());
                scope.spawn_or_drop("thread control", async move {
                    if let Some(event) = channel
                        .on_thread_control(&community, &payload, &task_scope)
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
        if let CommunityEnvelope::Control(
            ref payload @ (ControlPayload::EventCreated { .. }
            | ControlPayload::EventUpdated { .. }
            | ControlPayload::EventDeleted { .. }
            | ControlPayload::EventRsvpChanged { .. }),
        ) = envelope
        {
            if let Some(ctx) = self.host.get().and_then(std::sync::Weak::upgrade) {
                crate::host::adapters::governance::HostGovernance::new(ctx)
                    .on_event_control(community_id, sender_pseudonym, payload)
                    .await;
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
