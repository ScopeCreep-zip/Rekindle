//! Inbound event ingress: gossip and DHT ValueChange routing
//! through the central enrich → state-effects → dedup → emit pipeline.

use tracing::debug;

use super::{state_effects, watches, SubscriptionManager};
use crate::gossip::GossipAdmission;
use rekindle_codec::community::envelope::CommunityEnvelope;

use super::events::{self, SubscriptionEvent};

impl SubscriptionManager {
    /// Route a verified gossip envelope. Called by the daemon's
    /// `InboundHandler`.
    ///
    /// Pipeline: rate limit → Lamport merge → `envelope_into_event` →
    /// state_effects → dedup → emit.
    ///
    /// Takes the canonical `CommunityEnvelope`. It used to take a
    /// postcard `GossipPayload` — this track's own near-copy — so a
    /// desktop peer's broadcast could not be represented here at all.
    pub fn on_gossip(
        &self,
        community_id: &str,
        sender_pseudonym: &str,
        envelope: CommunityEnvelope,
        lamport_ts: u64,
    ) {
        debug!(
            community = community_id,
            sender = %sender_pseudonym,
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
        if !self.admit(community_id, sender_pseudonym, lamport_ts) {
            return;
        }

        // `None` for payloads that are not subscriber-facing — Mutual
        // Aid watch relays and the media/transport plane. Handled by
        // their own subsystems, not dropped silently here.
        if let Some(event) =
            crate::payload::gossip::envelope_into_event(envelope, community_id, sender_pseudonym)
        {
            self.process_event(event);
        }
    }

    /// The sender's per-mesh gossip rate limit and our Lamport merge: the
    /// admission [`Self::on_gossip`] runs, and a `HeadAdvanced` hint's
    /// before it is relayed (register S7l). `false` when over the floor.
    pub fn admit(&self, community_id: &str, sender_pseudonym: &str, lamport_ts: u64) -> bool {
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
            GossipAdmission::Accept => true,
            GossipAdmission::RateLimited => {
                debug!(
                    community = community_id,
                    sender = %sender_pseudonym,
                    "sub: gossip dropped — sender over rate floor"
                );
                false
            }
        }
    }

    /// Central event processing pipeline: enrich → state effects → dedup → emit.
    ///
    /// Single point of emission for ALL events regardless of source tier
    /// (watch, gossip, poll, or direct construction in on_value_change).
    pub(super) fn process_event(&self, mut event: SubscriptionEvent) {
        // Enrich: decrypt message bodies if MEK available, resolve display names
        self.enrich(&mut event);
        let now_ms = self.clock.now_ms();

        // Apply state side-effects (unread, typing, presence, voice)
        let extra_events = state_effects::apply(&mut self.state.write(), &event, now_ms);

        // Dedup gate: suppress duplicates from parallel tiers
        if self.dedup.write().check(&event, now_ms) {
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
                if let Some(friend) = guard
                    .as_ref()
                    .and_then(|s| s.friends.get(peer_key.as_str()))
                {
                    *sender_name = Some(friend.display_name.clone());
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
            // PATH 3: a channel segment record's slot changed. The Host
            // reads the slot and ingests each post through the D26 gate,
            // publishing `ChannelMessageEvent::New` for what it admits; an
            // event from here, before the gate, would show a removed
            // member's write (A/B finding R2-4).
            watches::WatchKind::ChannelRecord { .. } => {}
        }
    }
}
