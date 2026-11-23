//! Plan GI3: the relayed reactions and `HeadAdvanced` hints, through the one
//! Host ingest both modes call (the daemon's `ingest_gossip` and
//! `on_head_advanced`, the desktop's relay dispatch). Each takes the Host's
//! community gate (`takes_traffic_from`) and the gossip meter
//! (`SubscriptionManager::admit`, the daemon's overlay; the desktop meters
//! every envelope before relay dispatch, `receiver_limits.rs`), and the UI
//! event is the Host's, journaled here, never emitted from gossip.

use rekindle_codec::community::envelope::{CommunityEnvelope, ControlPayload};
use rekindle_types::id::PseudonymKey;

use super::HostGovernance;

impl HostGovernance {
    /// The gossip meter's admission of `sender` (with our Lamport merge):
    /// the daemon's `SubscriptionManager::admit`; with no manager (the
    /// desktop, whose relay ingress metered the envelope already) admitted.
    pub(crate) fn meter_admits(&self, community_id: &str, sender: &str, lamport_ts: u64) -> bool {
        (self.ctx.subscriptions.read().as_ref())
            .is_none_or(|sub| sub.admit(community_id, sender, lamport_ts))
    }

    /// A relayed `ReactionAdded`/`ReactionRemoved` from `sender` (the
    /// envelope's verified signer): nothing from traffic we do not take
    /// (removed, departed author), nothing over the meter, nothing naming a
    /// reactor other than its signer (no member reacts as another); then the
    /// `SocialEvent` is journaled. Any other payload is ignored.
    pub fn ingest_reaction(
        &self,
        community_id: &str,
        sender: &str,
        lamport_ts: u64,
        payload: &ControlPayload,
    ) {
        let (ControlPayload::ReactionAdded {
            reactor_pseudonym: reactor,
            ..
        }
        | ControlPayload::ReactionRemoved {
            reactor_pseudonym: reactor,
            ..
        }) = payload
        else {
            return;
        };
        if !self.takes_traffic_from(community_id, &PseudonymKey::from_hex_lossy(sender)) {
            tracing::debug!(community = %community_id, "reaction from traffic we do not take");
            return;
        }
        if !self.meter_admits(community_id, sender, lamport_ts) {
            return;
        }
        if reactor != sender {
            tracing::debug!(community = %community_id, "reaction names someone other than its signer");
            return;
        }
        let envelope = CommunityEnvelope::control(payload.clone());
        if let Some(event) =
            rekindle_transport::payload::gossip::envelope_into_event(envelope, community_id, sender)
        {
            self.ctx.publish_event(event);
        }
    }

    /// A relayed `HeadAdvanced` hint from `sender`: the community gate, then
    /// [`Self::on_head_advanced`] with the meter as its `admit`. `true` when
    /// it passed, so the caller relays it one hop further (gated before it
    /// is relayed, register S7l).
    pub fn ingest_head_advanced(&self, community_id: &str, sender: &str, hint: &[u8]) -> bool {
        if !self.takes_traffic_from(community_id, &PseudonymKey::from_hex_lossy(sender)) {
            tracing::debug!(community = %community_id, "head hint from traffic we do not take");
            return false;
        }
        self.on_head_advanced(community_id, sender, hint, |relayer| {
            self.meter_admits(community_id, relayer, 0)
        })
    }
}

#[cfg(test)]
#[path = "relayed_ingest_tests.rs"]
mod tests;
