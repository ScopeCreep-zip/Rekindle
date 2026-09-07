//! Inbound event ingress: gossip, DM, and DHT ValueChange routing
//! through the central enrich → state-effects → dedup → emit pipeline.

use tracing::debug;

use super::{state_effects, watches, SubscriptionManager};
use crate::gossip::GossipAdmission;
use crate::payload::dm::DmPayload;
use crate::payload::gossip::GossipPayload;

use super::events::{self, SubscriptionEvent};

impl SubscriptionManager {
    /// Route a gossip payload. Called by the daemon's InboundHandler.
    ///
    /// Pipeline: rate limit → Lamport merge → payload.into_event() →
    /// state_effects → dedup → emit
    pub fn on_gossip(
        &self,
        community_id: &str,
        sender_pseudonym: &str,
        payload: GossipPayload,
        lamport_ts: u64,
    ) {
        debug!(
            community = community_id,
            sender = &sender_pseudonym[..12.min(sender_pseudonym.len())],
            lamport = lamport_ts,
            "sub: on_gossip"
        );

        // Receiver-side admission. These are the two halves of the mesh
        // that were built but never consulted: `GossipMesh` has carried a
        // `rate_limiter` and a `clock` since it was written and nothing
        // called either, so the daemon had no flood protection (the
        // desktop enforces it in `receiver_limits.rs`) and never advanced
        // its Lamport clock from received traffic — its own sends were
        // ordered against a clock that only ever counted itself.
        //
        // The guard is scoped so it drops before `process_event`, which
        // re-enters the manager.
        let admission = {
            let mut meshes = self.meshes().write();
            match meshes.get_mut(community_id) {
                Some(mesh) => mesh.admit_gossip(sender_pseudonym, lamport_ts),
                // Gossip can arrive before `ensure_mesh` has run for this
                // community. Nothing to meter it against, and the
                // signature was already verified upstream, so admit it.
                None => GossipAdmission::Accept,
            }
        };
        match admission {
            GossipAdmission::Accept => {}
            GossipAdmission::RateLimited => {
                debug!(
                    community = community_id,
                    sender = &sender_pseudonym[..12.min(sender_pseudonym.len())],
                    "sub: gossip dropped — sender over rate floor"
                );
                return;
            }
            GossipAdmission::LamportDrift => {
                debug!(
                    community = community_id,
                    sender = &sender_pseudonym[..12.min(sender_pseudonym.len())],
                    lamport = lamport_ts,
                    "sub: gossip dropped — Lamport drift beyond cap"
                );
                return;
            }
        }

        let event = payload.into_event(community_id, sender_pseudonym);
        self.process_event(event);
    }

    /// Route a DM payload. Called by the daemon's InboundHandler.
    ///
    /// Pipeline: payload.into_event() → state_effects → dedup → emit
    pub fn on_dm(&self, sender_key: &str, payload: DmPayload, timestamp: u64) {
        debug!(
            sender = &sender_key[..12.min(sender_key.len())],
            timestamp, "sub: on_dm"
        );
        // W16.4 — call signaling and DM invites return None (they
        // surface via TransportNotification, not SubscriptionEvent).
        // The receive dispatch (W16.7) routes them to the call state
        // machine before reaching this layer.
        if let Some(event) = payload.into_event(sender_key, timestamp) {
            self.process_event(event);
        }
    }

    /// Central event processing pipeline: enrich → state effects → dedup → emit.
    ///
    /// Single point of emission for ALL events regardless of source tier
    /// (watch, gossip, poll, or direct construction in on_value_change).
    pub(super) fn process_event(&self, mut event: SubscriptionEvent) {
        // Enrich: decrypt message bodies if MEK available, resolve display names
        self.enrich(&mut event);

        // Apply state side-effects (unread, typing, presence, voice)
        let extra_events = state_effects::apply(&mut self.state.write(), &event);

        // Dedup gate: suppress duplicates from parallel tiers
        if self.dedup.write().check(&event) {
            let _ = self.event_tx.send(event);
        }

        // Emit any additional events (e.g., UnreadChanged)
        for extra in extra_events {
            let _ = self.event_tx.send(extra);
        }
    }

    /// Enrich an event with decrypted message bodies and resolved display names.
    ///
    /// Channel messages: attempt MEK decrypt if body is None.
    /// DMs: resolve sender_name from session friend list.
    /// All other events: no-op.
    fn enrich(&self, event: &mut SubscriptionEvent) {
        match event {
            SubscriptionEvent::ChannelMessage(events::ChannelMessageEvent::New {
                community,
                body,
                ..
            }) if body.is_none() => {
                // Attempt decrypt from MEK cache.
                // The full decrypt requires reading the ciphertext from DHT and
                // decrypting with the cached MEK. For gossip-originated events,
                // the ciphertext is not in the event — it's in the DHT record.
                // The enrichment reads the channel DhtLog entry and decrypts.
                //
                // For now: leave body as None. The TUI will show the message
                // metadata (sender, timestamp) and the body will be populated
                // on the next history load or when the poll tier refreshes.
                // Full inline decrypt is wired when QueryEngine is accessible here.
                let _ = community; // suppress unused warning until decrypt is wired
            }
            SubscriptionEvent::ChannelMessage(
                events::ChannelMessageEvent::DirectMessageReceived {
                    peer_key,
                    sender_name,
                    ..
                },
            ) if sender_name.is_none() => {
                // Resolve display name from friend list in session
                let guard = self.session.read();
                if let Some(ref session) = *guard {
                    if let Some(request) = session
                        .pending_friend_requests
                        .iter()
                        .find(|r| r.public_key == *peer_key)
                    {
                        *sender_name = Some(request.display_name.clone());
                    }
                }
            }
            _ => {}
        }
    }

    /// Route a DHT ValueChange by record key.
    pub fn on_value_change(
        &self,
        record_key: &str,
        changed_subkeys: Vec<u32>,
        _first_value: Option<Vec<u8>>,
    ) {
        let watch_kind = self.watches.read().get(record_key).map(|e| e.kind.clone());
        let Some(kind) = watch_kind else {
            self.process_event(SubscriptionEvent::Network(
                events::NetworkEvent::ValueChanged {
                    record_key: record_key.into(),
                    changed_subkeys,
                },
            ));
            return;
        };

        match kind {
            watches::WatchKind::FriendInbox => {
                debug!(record_key, subkeys = ?changed_subkeys, "friend inbox changed");
                // Update pending count from session
                let pending_count = self.session.read().as_ref().map_or(0, |s| {
                    u32::try_from(s.pending_friend_requests.len()).unwrap_or(u32::MAX)
                });
                self.state.write().unread.friend_requests = pending_count;
                let count = pending_count;
                self.process_event(SubscriptionEvent::UnreadChanged {
                    context: events::UnreadContext::FriendRequests,
                    count,
                });
            }
            watches::WatchKind::DmLog { peer_key } => {
                debug!(peer = %peer_key, "DM log changed");
                let count = self.state.write().unread.increment_dm(&peer_key);
                self.process_event(SubscriptionEvent::ChannelMessage(
                    events::ChannelMessageEvent::DirectMessageReceived {
                        peer_key: peer_key.clone(),
                        timestamp: rekindle_utils::timestamp_ms(),
                        sender_name: None, // enriched from friend list
                        body: None,        // enriched by reading DhtLog
                    },
                ));
                self.process_event(SubscriptionEvent::UnreadChanged {
                    context: events::UnreadContext::Dm { peer_key },
                    count,
                });
            }
            watches::WatchKind::GovernanceManifest { community } => {
                for subkey in &changed_subkeys {
                    let event = match *subkey {
                        crate::payload::dht_types::MANIFEST_METADATA => {
                            events::GovernanceEvent::MetadataChanged {
                                community: community.clone(),
                            }
                        }
                        crate::payload::dht_types::MANIFEST_CHANNELS => {
                            events::GovernanceEvent::ChannelsChanged {
                                community: community.clone(),
                            }
                        }
                        crate::payload::dht_types::MANIFEST_ROLES => {
                            events::GovernanceEvent::RolesChanged {
                                community: community.clone(),
                            }
                        }
                        crate::payload::dht_types::MANIFEST_BANS => {
                            events::GovernanceEvent::BansChanged {
                                community: community.clone(),
                            }
                        }
                        crate::payload::dht_types::MANIFEST_INVITES => {
                            events::GovernanceEvent::InvitesChanged {
                                community: community.clone(),
                            }
                        }
                        other => events::GovernanceEvent::GovernanceSubkeyUpdated {
                            community: community.clone(),
                            subkey_index: other,
                            lamport_ts: 0,
                        },
                    };
                    self.process_event(SubscriptionEvent::Governance(event));
                }
            }
            watches::WatchKind::MemberRegistry { community } => {
                for subkey in &changed_subkeys {
                    match *subkey {
                        crate::payload::dht_types::REGISTRY_MEMBER_INDEX => {
                            debug!(community = %community, "member index changed");
                            // The daemon re-reads the member list on this signal.
                        }
                        crate::payload::dht_types::REGISTRY_MEK_VAULT => {
                            // Read the current max generation from the mek_cache
                            let generation = self
                                .mek_cache
                                .read()
                                .snapshot(&community)
                                .iter()
                                .map(|e| e.generation)
                                .max()
                                .unwrap_or(0);
                            debug!(community = %community, generation, "MEK vault changed");
                            self.process_event(SubscriptionEvent::Crypto(
                                events::CryptoEvent::MekRotated {
                                    community: community.clone(),
                                    channel: None,
                                    generation,
                                    rotator_pseudonym: None,
                                },
                            ));
                        }
                        crate::payload::dht_types::REGISTRY_MODERATION_QUEUE => {
                            debug!(community = %community, "moderation queue changed");
                        }
                        _ => {}
                    }
                }
            }
            watches::WatchKind::JoinInbox { community } => {
                debug!(community = %community, subkeys = ?changed_subkeys, "join inbox changed");
                // The daemon's inbox processor is triggered by this signal.
            }
            watches::WatchKind::ChannelLog {
                community,
                channel_id,
                member_pseudonym,
            } => {
                let count = self
                    .state
                    .write()
                    .unread
                    .increment_channel(&community, &channel_id);
                self.process_event(SubscriptionEvent::ChannelMessage(
                    events::ChannelMessageEvent::New {
                        community: community.clone(),
                        channel: channel_id.clone(),
                        message_id: String::new(), // resolved by the reader
                        sender_pseudonym: member_pseudonym,
                        sequence: 0,
                        timestamp: rekindle_utils::timestamp_ms(),
                        body: None,              // enriched by decrypt stage
                        reply_to_sequence: None, // enriched by decrypt stage
                    },
                ));
                self.process_event(SubscriptionEvent::UnreadChanged {
                    context: events::UnreadContext::Channel {
                        community,
                        channel: channel_id,
                    },
                    count,
                });
            }
        }
    }
}
