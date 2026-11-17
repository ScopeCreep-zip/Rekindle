//! The daemon's community gossip ingest (`InboundHandler::on_gossip`): the
//! own-removal and departed-author gate, then each control payload to its
//! Host owner or the `SubscriptionManager`; and the gate and channel-post
//! hand-off the handler's other gossip paths share.

use rekindle_codec::community::envelope::{CommunityEnvelope, ControlPayload};
use tracing::{debug, info};

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

        // The membership control messages (plan E3.1 B2e): a JoinAccepted's
        // community key, members and slot seed are installed before the
        // pending join below resumes.
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

        // A thread opened, archived or replied to (plan E3.1 E3f): stored by
        // the Host's channel adapter, which returns the event to publish (a
        // reply's carries the body it read from the thread record; the
        // transport maps no thread payload).
        if let CommunityEnvelope::Control(
            ref payload @ (ControlPayload::ThreadCreated { .. }
            | ControlPayload::ThreadArchived { .. }
            | ControlPayload::ThreadMessageReceived { .. }),
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

        // Tier 2: If this is a JoinAccepted for a pending join, complete oneshot.
        // Check BEFORE forwarding to SubscriptionManager (which takes ownership).
        if let CommunityEnvelope::Control(ControlPayload::JoinAccepted {
            slot_index: Some(slot),
            ..
        }) = &envelope
        {
            let mut pending = self.pending_joins.lock();
            if let Some((tx, _)) = pending.remove(community_id) {
                let _ = tx.send(*slot);
                info!(
                    community = community_id,
                    slot, "join approved via direct notification (tier 2)"
                );
            }
        }

        // A channel post (plan E3.1 F3): the Host's ingest fetches, gates
        // and stores it once the mesh admits its sender.
        let notified = match &envelope {
            CommunityEnvelope::MessageNotification {
                channel_id,
                message_id,
                author_pseudonym,
                subkey_index,
                sequence,
                ..
            } if author_pseudonym.eq_ignore_ascii_case(sender_pseudonym) => Some((
                channel_id.clone(),
                message_id.clone(),
                *subkey_index,
                *sequence,
            )),
            _ => None,
        };
        let admitted = match *self.subscriptions.read() {
            Some(ref sub_mgr) => {
                sub_mgr.on_gossip(community_id, sender_pseudonym, envelope, lamport_ts)
            }
            None => false,
        };
        if let (true, Some(post)) = (admitted, notified) {
            self.ingest_post(community_id, sender_pseudonym, post);
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

    /// Hand a notified channel post to the Host's ingest (plan E3.1 F3).
    fn ingest_post(&self, community_id: &str, author: &str, post: (String, String, u32, u64)) {
        let Some(ctx) = self.host.get().and_then(std::sync::Weak::upgrade) else {
            return;
        };
        let (community, author) = (community_id.to_owned(), author.to_owned());
        let (channel, message_id, subkey, sequence) = post;
        let scope = ctx.unlock_scope_or_closed();
        scope.spawn_or_drop("channel post ingest", async move {
            let host = crate::host::adapters::channel::HostChannel::new(&ctx);
            let ingested = host
                .ingest_notified(&community, &channel, subkey, &message_id, &author, sequence)
                .await;
            if let Err(error) = ingested {
                debug!(%error, community, channel, "received channel post not ingested");
            }
        });
    }
}
