//! Write paths into daemon state.
//!
//! Split by where the write lands:
//!
//! - Runtime-only (cached governance state, open records, overflow
//!   chain) goes to [`crate::daemon::community_runtime`] and is not
//!   persisted — it is rebuilt from the DHT.
//! - Membership changes go to the identity's stores and its session
//!   projection through `host::session` (plan D1), because losing them
//!   means losing the slot.

use rekindle_governance::state::GovernanceState;
use rekindle_governance_runtime::deps::CommunityInsert;
use rekindle_transport::session::CommunityMembership as SessionMembership;

use super::DaemonGovernanceAdapter;

impl DaemonGovernanceAdapter<'_> {
    pub(super) fn set_governance_state_impl(&self, community_id: &str, state: GovernanceState) {
        // Record each channel's record key before the state lands.
        // `ChannelCreated.record_key` is the canonical location, but the
        // session copy is what `setup_community_watches` and the leave path
        // read — and only the *creator's* `insert_community` ever seeded
        // it, so a joined community had no channel watches at all.
        crate::host::session::record_channels(
            &self.ctx.session,
            &self.ctx.db,
            community_id,
            &state,
        );
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
    /// Stored, because a counter that restarts at zero after a daemon
    /// restart would emit entries that lose every LWW merge against our
    /// own earlier writes.
    pub(super) fn next_governance_lamport_impl(
        &self,
        community_id: &str,
    ) -> Result<u64, rekindle_types::lamport::LamportError> {
        crate::host::session::next_clock(&self.ctx.session, &self.ctx.db, community_id)
    }

    /// Record a freshly created community (origin flow).
    ///
    /// Stores the membership with the origin's owner keypairs and seeds
    /// the runtime cache with the genesis governance state, so the very
    /// first read after creation does not have to go back to the DHT.
    pub(super) fn insert_community_impl(&self, community: CommunityInsert) {
        let slot_seed = hex::decode(&community.slot_seed_hex)
            .ok()
            .and_then(|b| <[u8; 32]>::try_from(b).ok());
        let membership = SessionMembership {
            governance_key: community.governance_key.clone(),
            pseudonym_key: community.my_pseudonym_hex.clone(),
            role_ids: community.creator_role_ids.clone(),
            registry_key: community.registry_key.clone(),
            // Origin always takes slot 0 of the genesis segment.
            slot_index: 0,
            community_name: community.name.clone(),
            slot_seed,
            channel_record_keys: std::collections::HashMap::new(),
            segment_index: Some(0),
            lamport_counter: community.governance_clock,
            mek_generation: community.mek.generation,
        };
        // The registry and governance owner keypairs grant no writer slot
        // under `o_cnt: 0` (members write their subkeys with slot keypairs),
        // but they are the records' owner credentials: the vault and the
        // community row keep them as the desktop does.
        let recorded = crate::host::session::record_membership(
            &self.ctx.session,
            &self.ctx.db,
            &self.ctx.vault,
            crate::host::session::NewMembership {
                membership,
                dht_owner_keypair: community.dht_owner_keypair,
                registry_keypair: community.registry_owner_keypair,
            },
        );
        if let Err(e) = recorded {
            tracing::error!(error = %e, community = %community.id, "created community not recorded");
            return;
        }
        // The genesis channel's record key, from the genesis state.
        self.set_governance_state_impl(&community.id, community.governance_state);
        self.insert_community_mek_impl(&community.id, &community.mek);
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
    /// adopt the slot index and role IDs the DHT knows for us.
    pub(super) fn apply_recovered_member_state_impl(
        &self,
        community_id: &str,
        subkey_index: u32,
        role_ids: &[u32],
    ) {
        crate::host::session::update_slot_and_roles(
            &self.ctx.session,
            &self.ctx.db,
            community_id,
            subkey_index,
            role_ids,
        );
    }
}
