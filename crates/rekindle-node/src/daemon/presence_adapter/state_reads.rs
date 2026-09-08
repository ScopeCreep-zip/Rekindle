//! Reads over session and runtime state.

use std::collections::{HashMap, HashSet};

use rekindle_presence::deps::{PresenceCredentials, SegmentDescriptor, SelfPresenceSnapshot};
use rekindle_types::id::{PseudonymKey, RoleId};

use super::DaemonPresenceAdapter;

impl DaemonPresenceAdapter {
    /// Our pseudonym for a community, or empty when we are not a member.
    ///
    /// The trait returns `String` rather than `Option`, so the empty
    /// string is the "not a member" signal — callers compare against it.
    pub(super) fn my_pseudonym_impl(&self, community_id: &str) -> String {
        self.ctx
            .session
            .read()
            .as_ref()
            .and_then(|s| s.community(community_id))
            .map(|m| m.pseudonym_key.clone())
            .unwrap_or_default()
    }

    pub(super) fn identity_display_name_impl(&self) -> String {
        self.ctx
            .session
            .read()
            .as_ref()
            .map(|s| s.identity.display_name.clone())
            .unwrap_or_default()
    }

    pub(super) fn our_route_blob_impl(&self) -> Option<Vec<u8>> {
        let node = self.transport()?;
        let blob = node.routes().read().route_blob()?.to_vec();
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

    pub(super) fn channel_ids_impl(&self, community_id: &str) -> Vec<String> {
        self.ctx
            .session
            .read()
            .as_ref()
            .and_then(|s| s.community(community_id))
            .map(|m| m.channel_record_keys.keys().cloned().collect())
            .unwrap_or_default()
    }

    pub(super) fn channel_log_keys_impl(&self, community_id: &str) -> Vec<(String, String)> {
        self.ctx
            .session
            .read()
            .as_ref()
            .and_then(|s| s.community(community_id))
            .map(|m| {
                m.channel_record_keys
                    .iter()
                    .map(|(channel, key)| (channel.clone(), key.clone()))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Members the last poll observed.
    ///
    /// This is the read that replaces `read_member_index` — one local
    /// map lookup against a roster whose every row was signature-checked
    /// when it was scanned, rather than a DHT read of a shared structure
    /// any member could have written.
    pub(super) fn member_count_impl(&self, community_id: &str) -> u32 {
        u32::try_from(self.ctx.community_runtime.member_count(community_id)).unwrap_or(u32::MAX)
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
                rekindle_transport::broadcast::dht_writes::derive_slot_keypair_str(
                    &seed,
                    membership.slot_index,
                )
                .ok()
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
            .map(|m| m.registry_key.clone());

        let mut out = Vec::new();
        if let Some(registry_key) = primary {
            out.push(SegmentDescriptor {
                segment_index: 0,
                registry_key,
            });
        }
        if let Some(state) = self.ctx.community_runtime.governance_state(community_id) {
            for seg in &state.segments {
                if seg.segment_index == 0 {
                    continue;
                }
                out.push(SegmentDescriptor {
                    segment_index: seg.segment_index,
                    registry_key: seg.registry_key.clone(),
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
            .community_runtime
            .governance_state(community_id)
            .map(|s| s.bans.iter().map(|p| hex::encode(p.0)).collect())
            .unwrap_or_default()
    }

    pub(super) fn member_roles_impl(&self, community_id: &str) -> HashMap<String, Vec<u32>> {
        self.ctx.community_runtime.member_roles(community_id)
    }

    pub(super) fn governance_role_assignments_impl(
        &self,
        community_id: &str,
    ) -> HashMap<PseudonymKey, HashSet<RoleId>> {
        self.ctx
            .community_runtime
            .governance_state(community_id)
            .map(|s| s.role_assignments.clone())
            .unwrap_or_default()
    }

    pub(super) fn my_role_ids_impl(&self, community_id: &str) -> Vec<u32> {
        self.ctx
            .session
            .read()
            .as_ref()
            .and_then(|s| s.community(community_id))
            .map(|m| m.role_ids.clone())
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
