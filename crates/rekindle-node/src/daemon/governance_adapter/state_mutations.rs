//! Write paths into daemon state.
//!
//! Split by where the write lands:
//!
//! - Runtime-only (cached governance state, open records, overflow
//!   chain) goes to [`crate::daemon::community_runtime`] and is not
//!   persisted — it is rebuilt from the DHT.
//! - Membership changes go to `Session.communities` and are persisted to
//!   `session.json`, because losing them means losing the slot.
//!
//! Every function drops its guard before returning; `save_session` is
//! called outside the lock for the same reason.

use rekindle_governance::state::GovernanceState;
use rekindle_governance_runtime::deps::CommunityInsert;
use rekindle_transport::session::CommunityMembership as SessionMembership;

use super::DaemonGovernanceAdapter;

impl DaemonGovernanceAdapter {
    pub(super) fn set_governance_state_impl(&self, community_id: &str, state: GovernanceState) {
        self.ctx
            .community_runtime
            .set_governance_state(community_id, state);
    }

    /// Advance the community's **governance** Lamport counter and return
    /// the new value.
    ///
    /// Not the gossip mesh clock (`GossipMesh::clock`) — that one orders
    /// epidemic broadcast, this one stamps governance entries for the
    /// CRDT merge. Two clocks on purpose; do not collapse them.
    ///
    /// Persisted, because a counter that restarts at zero after a daemon
    /// restart would emit entries that lose every LWW merge against our
    /// own earlier writes.
    pub(super) fn increment_lamport_impl(&self, community_id: &str) -> u64 {
        let next = {
            let mut guard = self.ctx.session.write();
            let Some(session) = guard.as_mut() else {
                return 0;
            };
            let Some(membership) = session.communities.get_mut(community_id) else {
                return 0;
            };
            membership.lamport_counter = membership.lamport_counter.saturating_add(1);
            membership.lamport_counter
        };
        self.persist_session();
        next
    }

    /// Record a freshly created community (origin flow).
    ///
    /// Builds the persisted membership from the boundary DTO and seeds
    /// the runtime cache with the genesis governance state, so the very
    /// first read after creation does not have to go back to the DHT.
    pub(super) fn insert_community_impl(&self, community: CommunityInsert) {
        let slot_seed = hex::decode(&community.slot_seed_hex)
            .ok()
            .and_then(|b| <[u8; 32]>::try_from(b).ok());

        let mut channel_record_keys = std::collections::HashMap::new();
        channel_record_keys.insert(
            community.channel_id_hex.clone(),
            community.channel_record_key.clone(),
        );

        let membership = SessionMembership {
            governance_key: community.governance_key.clone(),
            pseudonym_key: community.my_pseudonym_hex.clone(),
            display_name: self.identity_display_name_impl(),
            role_ids: community.creator_role_ids.clone(),
            registry_key: community.registry_key.clone(),
            // Origin always takes slot 0 of the genesis segment.
            slot_index: 0,
            community_name: community.name.clone(),
            slot_seed,
            channel_record_keys,
            // The origin flow does not mint a mailbox or join inbox; the
            // v2.0 join is self-sovereign and needs neither.
            community_mailbox_key: String::new(),
            join_inbox_key: String::new(),
            is_operator: true,
            governance_keypair_label: None,
            segment_index: Some(0),
            lamport_counter: community.lamport_counter,
            mek_generation: community.mek.generation,
        };

        {
            let mut guard = self.ctx.session.write();
            if let Some(session) = guard.as_mut() {
                session.join_community(membership);
            }
        }
        self.persist_session();

        self.ctx
            .community_runtime
            .set_governance_state(&community.id, community.governance_state);
        self.insert_community_mek_impl(&community.id, &community.mek);
    }

    pub(super) fn mark_open_channel_record_impl(&self, community_id: &str, record_key: String) {
        self.ctx
            .community_runtime
            .mark_open_record(community_id, record_key);
    }

    pub(super) fn track_open_dht_records_impl(&self, community_id: &str, keys: &[String]) {
        self.ctx
            .community_runtime
            .mark_open_records(community_id, keys);
    }

    pub(super) fn register_governance_overflow_keys_impl(
        &self,
        community_id: &str,
        keys: &[String],
    ) {
        self.ctx
            .community_runtime
            .register_overflow_keys(community_id, keys);
    }

    /// Apply state recovered from the registry for the logged-in user:
    /// fill in a missing slot index, and adopt role IDs the DHT knows
    /// about that we do not.
    pub(super) fn apply_recovered_member_state_impl(
        &self,
        community_id: &str,
        subkey_index: u32,
        role_ids: &[u32],
    ) {
        let changed = {
            let mut guard = self.ctx.session.write();
            let Some(session) = guard.as_mut() else {
                return;
            };
            let Some(membership) = session.communities.get_mut(community_id) else {
                return;
            };
            let mut changed = false;
            if membership.slot_index != subkey_index {
                membership.slot_index = subkey_index;
                changed = true;
            }
            if membership.role_ids != role_ids {
                membership.role_ids = role_ids.to_vec();
                changed = true;
            }
            changed
        };
        if changed {
            self.persist_session();
        }
    }

    /// Write `session.json`.
    ///
    /// Failures are logged rather than propagated: the callers are trait
    /// methods returning `()`, and losing the persist is recoverable
    /// (the state is still correct in memory and will be rewritten on
    /// the next mutation), whereas panicking in a governance path is
    /// not.
    pub(super) fn persist_session(&self) {
        let snapshot = self.ctx.session.read().clone();
        let Some(session) = snapshot else {
            return;
        };
        if let Err(e) = session.save(&self.ctx.session_path) {
            tracing::warn!(
                error = %e,
                path = %self.ctx.session_path.display(),
                "governance adapter: session persist failed"
            );
        }
    }
}
