//! Reads over session and runtime state.

use std::collections::HashSet;

use rekindle_presence::deps::{PresenceCredentials, SegmentDescriptor, SelfPresenceSnapshot};

use super::DaemonPresenceAdapter;

impl DaemonPresenceAdapter {
    /// Our pseudonym for a community, or empty when we are not a member.
    ///
    /// The trait returns `String` rather than `Option`, so the empty
    /// string is the "not a member" signal — callers compare against it.
    pub(super) fn my_pseudonym_impl(&self, community_id: &str) -> String {
        self.ctx.my_pseudonym(community_id)
    }

    pub(super) fn our_route_blob_impl(&self) -> Option<Vec<u8>> {
        let node = self.transport()?;
        let blob = node.personal_route_blob()?;
        // An allocated-but-empty route is not publishable: a peer that
        // imported it would have nothing to send to.
        (!blob.is_empty()).then_some(blob)
    }

    /// The daemon has no tray or status selector, so presence is
    /// "online" whenever it is running. `presence::set_status` writes
    /// the profile record directly and does not route through here.
    pub(super) fn current_status_impl() -> String {
        "online".to_string()
    }

    pub(super) fn presence_credentials_impl(
        &self,
        community_id: &str,
    ) -> Option<PresenceCredentials> {
        let guard = self.ctx.session.read();
        let membership = guard.as_ref()?.community(community_id)?;
        Some(PresenceCredentials {
            my_pseudonym_hex: membership.pseudonym_key.clone(),
            my_subkey_index: Some(membership.slot_index),
            // Derived on demand from the seed rather than stored: the
            // seed is the durable secret and the keypair is a pure
            // function of it plus the slot.
            slot_keypair_str: membership.slot_seed.and_then(|seed| {
                rekindle_crypto::keypair_text::slot_keypair_text(&seed, membership.slot_index).ok()
            }),
            slot_seed_hex: membership.slot_seed.map(hex::encode),
            my_segment_index: membership.segment_index.unwrap_or(0),
        })
    }

    /// Segment 0 from the membership, plus every `SegmentAdded` the
    /// governance CRDT has merged.
    pub(super) fn segment_descriptors_impl(&self, community_id: &str) -> Vec<SegmentDescriptor> {
        let primary = self
            .ctx
            .session
            .read()
            .as_ref()
            .and_then(|s| s.community(community_id))
            .map(|m| (m.registry_key.clone(), m.anchor.clone()));

        let mut out = Vec::new();
        if let Some((registry_key, anchor)) = primary {
            out.push(SegmentDescriptor {
                segment_index: 0,
                registry_key,
                // Segment 0's governance record is the community's own.
                anchor,
                slot_range_start: 0,
                slot_range_end: rekindle_types::dht_layout::SLOTS_PER_SEGMENT,
            });
        }
        if let Some(state) = self.ctx.communities.governance_state(community_id) {
            for seg in &state.segments {
                if seg.segment_index == 0 {
                    continue;
                }
                out.push(SegmentDescriptor {
                    segment_index: seg.segment_index,
                    registry_key: seg.registry_key.clone(),
                    // Carried rather than dropped: the merged segment
                    // entry has it, and the type can now hold it.
                    anchor: seg.anchor.clone(),
                    slot_range_start: seg.slot_range_start,
                    slot_range_end: seg.slot_range_end,
                });
            }
        }
        out.sort_by_key(|s| s.segment_index);
        out
    }

    /// Banned pseudonyms from merged governance — the CRDT is the
    /// authority, so this is a local read, not a DHT one.
    pub(super) fn governance_bans_impl(&self, community_id: &str) -> HashSet<String> {
        self.ctx
            .communities
            .governance_state(community_id)
            .map(|s| s.bans.iter().map(|p| hex::encode(p.0)).collect())
            .unwrap_or_default()
    }

    /// The daemon publishes no profile fields of its own — no avatar,
    /// bio, badges or RSVPs — so its presence row carries identity and
    /// liveness only. A headless node with a cosmetic profile would be
    /// inventing data no user entered.
    pub(super) fn self_presence_snapshot_impl() -> SelfPresenceSnapshot {
        SelfPresenceSnapshot::default()
    }
}
