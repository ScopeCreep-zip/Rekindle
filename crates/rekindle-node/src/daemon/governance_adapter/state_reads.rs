//! Read paths: identity, membership, online members, open records.
//!
//! Every function here takes a lock, clones what it needs, and drops the
//! guard before returning. `parking_lot` guards are `!Send`, and the
//! trait methods these back are called from async contexts.

use rekindle_governance::state::GovernanceState;
use rekindle_governance_runtime::deps::{
    CommunityMembership, OnlineMemberSnapshot, UserStatusKind,
};

use rekindle_transport::session::CommunityMembership as SessionMembership;

use super::DaemonGovernanceAdapter;

use rekindle_protocol::dht::community::member_registry::SLOTS_PER_SEGMENT;

impl DaemonGovernanceAdapter<'_> {
    /// Ed25519 identity secret, or `None` while the daemon is locked.
    pub(super) fn identity_secret_impl(&self) -> Option<[u8; 32]> {
        self.ctx.signing_key.read().as_ref().map(|k| *k.as_bytes())
    }

    pub(super) fn identity_display_name_impl(&self) -> String {
        self.ctx
            .session
            .read()
            .as_ref()
            .map(|s| s.identity.display_name.clone())
            .unwrap_or_default()
    }

    /// The daemon has no per-process status toggle the way the desktop
    /// tray does — `presence::set_status` writes straight to the profile
    /// record. A running daemon is by definition reachable, so it
    /// reports `Online`; a locked one cannot answer at all, and callers
    /// of this method have already passed the unlock gate.
    pub(super) fn identity_status_impl() -> UserStatusKind {
        UserStatusKind::Online
    }

    /// Our current private-route blob, or empty when no route is
    /// allocated yet. Empty rather than an error because the trait
    /// returns a bare `Vec<u8>`; callers write it into presence records
    /// where an empty blob simply means "not reachable yet".
    pub(super) fn our_route_blob_impl(&self) -> Vec<u8> {
        let Ok(transport) = self.transport() else {
            return Vec::new();
        };
        let routes = transport.routes();
        let guard = routes.read();
        guard.route_blob().map(<[u8]>::to_vec).unwrap_or_default()
    }

    /// Project the daemon's persisted membership onto the boundary
    /// snapshot the runtime crate consumes.
    ///
    /// The two `CommunityMembership` types are deliberately distinct:
    /// this one is `session.json`'s record with non-optional fields, the
    /// other is the all-`Option` runtime view. Converting here is the
    /// adapter doing its job, not duplication to collapse.
    pub(super) fn community_membership_impl(
        &self,
        community_id: &str,
    ) -> Option<CommunityMembership> {
        let guard = self.ctx.session.read();
        let membership = guard.as_ref()?.communities.get(community_id)?;

        // The slot keypair is derived, not stored: the seed plus our
        // global slot index reproduce it, and the seed is what lives in
        // the keyring. Global slot = segment * SLOTS_PER_SEGMENT + local
        // index (architecture §15.2/§8.3); for the genesis segment the
        // two are equal, which is what `None` falls back to.
        //
        // Goes through transport's string-typed helper rather than
        // rekindle-protocol directly: this crate must not name a
        // `veilid_core::KeyPair`, and does not depend on that crate.
        let global_slot =
            membership.segment_index.unwrap_or(0) * SLOTS_PER_SEGMENT + membership.slot_index;
        let slot_keypair = membership.slot_seed.as_ref().and_then(|seed| {
            rekindle_transport::broadcast::dht_writes::derive_slot_keypair_str(seed, global_slot)
                .ok()
        });

        Some(CommunityMembership {
            governance_key: Some(membership.governance_key.clone()),
            member_registry_key: Some(membership.registry_key.clone()),
            my_pseudonym_hex: Some(membership.pseudonym_key.clone()),
            my_subkey_index: Some(membership.slot_index),
            my_segment_index: membership.segment_index,
            slot_keypair,
            slot_seed_hex: membership.slot_seed.as_ref().map(hex::encode),
            // Operators hold the governance keypair in the OS keyring
            // under `governance_keypair_label`; it is not carried in
            // session.json, so it is fetched by the DHT paths that
            // actually need a writer rather than eagerly here.
            dht_owner_keypair: None,
            lamport_counter: membership.lamport_counter,
            channel_log_keys: self.channel_record_keys_impl(community_id, membership),
            channel_ids: self.channel_ids_impl(community_id, membership),
            mek_generation: membership.mek_generation,
        })
    }

    pub(super) fn governance_state_impl(&self, community_id: &str) -> Option<GovernanceState> {
        self.ctx.community_runtime.governance_state(community_id)
    }

    /// Channel-id → segment-0 record key, from merged governance.
    ///
    /// `ChannelCreated.record_key` is the canonical location: it is CRDT
    /// state every peer merged, so a joiner has it the moment the merge
    /// lands. The session copy is a cache that only the creator's own
    /// `insert_community` seeds, which is why a joined community used to
    /// have an empty map and no way to send.
    ///
    /// Session entries are folded in underneath rather than ignored —
    /// they cover the window before the first merge completes.
    fn channel_record_keys_impl(
        &self,
        community_id: &str,
        membership: &SessionMembership,
    ) -> std::collections::HashMap<String, String> {
        let mut out = membership.channel_record_keys.clone();
        if let Some(state) = self.ctx.community_runtime.governance_state(community_id) {
            for (channel_id, channel) in &state.channels {
                if channel.record_key.is_empty() {
                    continue;
                }
                out.insert(hex::encode(channel_id.0), channel.record_key.clone());
            }
        }
        out
    }

    fn channel_ids_impl(&self, community_id: &str, membership: &SessionMembership) -> Vec<String> {
        self.channel_record_keys_impl(community_id, membership)
            .into_keys()
            .collect()
    }

    /// Snapshot the gossip overlay's online members.
    ///
    /// Reads through `SubscriptionManager`, which owns the meshes on
    /// this track. Returns empty when the manager is absent (pre-unlock)
    /// or the community has no mesh yet — both mean "nobody known
    /// online", which is what callers do with an empty Vec anyway.
    pub(super) fn online_members_impl(&self, community_id: &str) -> Vec<OnlineMemberSnapshot> {
        let guard = self.ctx.subscriptions.read();
        let Some(manager) = guard.as_ref() else {
            return Vec::new();
        };
        let meshes = manager.meshes().read();
        let Some(mesh) = meshes.get(community_id) else {
            return Vec::new();
        };
        mesh.online_members
            .iter()
            .map(|(pseudonym_hex, member)| OnlineMemberSnapshot {
                pseudonym_hex: pseudonym_hex.clone(),
                status: member.status.clone(),
                route_blob: member.route_blob.clone(),
                last_seen: member.last_seen,
            })
            .collect()
    }

    pub(super) fn open_record_keys_impl(&self, community_id: &str) -> Vec<String> {
        self.ctx.community_runtime.open_record_keys(community_id)
    }

    /// Every joined community's `(id, governance_key)`.
    ///
    /// On this track `governance_key` is non-optional in the persisted
    /// record and doubles as the map key, so unlike the desktop adapter
    /// there is no v1.0-legacy row to skip.
    pub(super) fn list_community_governance_targets_impl(&self) -> Vec<(String, String)> {
        let guard = self.ctx.session.read();
        let Some(session) = guard.as_ref() else {
            return Vec::new();
        };
        session
            .communities
            .values()
            .map(|m| (m.governance_key.clone(), m.governance_key.clone()))
            .collect()
    }

    /// Every joined community's `(id, registry_key, my_pseudonym_hex)`.
    pub(super) fn list_registries_with_my_pseudonym_impl(
        &self,
    ) -> Vec<(String, String, Option<String>)> {
        let guard = self.ctx.session.read();
        let Some(session) = guard.as_ref() else {
            return Vec::new();
        };
        session
            .communities
            .values()
            .filter(|m| !m.registry_key.is_empty())
            .map(|m| {
                (
                    m.governance_key.clone(),
                    m.registry_key.clone(),
                    Some(m.pseudonym_key.clone()),
                )
            })
            .collect()
    }

    /// Channel message-record keys for a community.
    pub(super) fn channel_log_keys_for_community_impl(&self, community_id: &str) -> Vec<String> {
        let guard = self.ctx.session.read();
        let Some(membership) = guard.as_ref().and_then(|s| s.communities.get(community_id)) else {
            return Vec::new();
        };
        self.channel_record_keys_impl(community_id, membership)
            .into_values()
            .collect()
    }

    pub(super) fn governance_overflow_keys_impl(&self, community_id: &str) -> Vec<String> {
        self.ctx.community_runtime.overflow_keys(community_id)
    }
}
