//! Gossip, permissions, background tasks and hydration.
//!
//! The remainder of the trait surface — everything that is neither a
//! plain state access nor a direct DHT call.

use rekindle_codec::community::envelope::CommunityEnvelope;
use rekindle_governance::state::GovernanceState;
use rekindle_governance_runtime::deps::{CommunityDhtOpenSetup, DiscoveredMember};
use rekindle_governance_runtime::GovernanceRuntimeError;
use rekindle_types::governance::GovernanceEntry;
use rekindle_types::id::PseudonymKey;

use super::DaemonGovernanceAdapter;

impl DaemonGovernanceAdapter<'_> {
    // ---------- Gossip ----------

    /// Fan a governance envelope out over the community mesh.
    ///
    /// Queued for the gossip worker. This used to accept only
    /// `Control(..)` and, within that, only the variants transport had a
    /// postcard helper for — everything else returned "this control
    /// variant has no transport gossip". Every variant now goes, in the
    /// Cap'n Proto form desktop peers read. See `daemon::gossip`.
    pub(super) fn send_to_mesh_impl(&self, community_id: &str, envelope: &CommunityEnvelope) {
        crate::daemon::gossip::send(&self.ctx.gossip_tx, community_id, envelope);
    }

    // ---------- Permissions ----------

    /// Reader-validates permission check against the merged CRDT state.
    ///
    /// No privileged shortcut for the daemon: it computes its own
    /// effective permissions exactly as any other peer would, from the
    /// governance state, and denies when the bits are absent. A daemon
    /// that trusted itself here would be a privileged node, which is the
    /// thing v2.0 removed.
    pub(super) fn require_permission_impl(
        &self,
        community_id: &str,
        perm_bits: u64,
    ) -> Result<(), GovernanceRuntimeError> {
        let Some(state) = self.governance_state_impl(community_id) else {
            return Err(GovernanceRuntimeError::Adapter(
                "governance state not loaded for permission check".into(),
            ));
        };
        let Some(membership) = self.community_membership_impl(community_id) else {
            return Err(GovernanceRuntimeError::Adapter(
                "not a member of this community".into(),
            ));
        };
        let Some(pseudonym_hex) = membership.my_pseudonym_hex else {
            return Err(GovernanceRuntimeError::Adapter(
                "no pseudonym for this community".into(),
            ));
        };
        let pseudonym_bytes: [u8; 32] = hex::decode(&pseudonym_hex)
            .ok()
            .and_then(|b| b.try_into().ok())
            .ok_or_else(|| GovernanceRuntimeError::Adapter("invalid pseudonym hex".into()))?;
        let pseudonym = PseudonymKey(pseudonym_bytes);

        let effective = rekindle_governance::permissions::compute_permissions(
            &pseudonym,
            None,
            &state,
            rekindle_utils::timestamp_secs(),
        );
        if rekindle_governance::permissions::has_capability(effective, perm_bits) {
            Ok(())
        } else {
            Err(GovernanceRuntimeError::PermissionDenied)
        }
    }

    // ---------- Join flow ----------

    /// `app_call` a peer through its advertised private route.
    ///
    /// **Unframed**, matching the desktop, which sends a raw `app_call`
    /// (`governance_adapter/dht.rs`). The daemon used to wrap the same
    /// bytes in a `TypeId::CommunityGovOp` frame and sign them as a
    /// `SignedPayload`, so the two shells could not have answered each
    /// other's peer calls — the frame is gone with the `GovernanceOp`
    /// mechanism it belonged to.
    ///
    /// The payload is the caller's business: this is the transport for
    /// the BootstrapBundle fetch (§14.4), which the architecture calls
    /// "convenience, not trust" — nothing here can be relied on for
    /// authority, and the bundle's contents are re-verified against the
    /// DHT regardless.
    pub(super) async fn app_call_peer_impl(
        &self,
        target_route_blob: &[u8],
        payload: Vec<u8>,
    ) -> Result<Vec<u8>, GovernanceRuntimeError> {
        let node = self.transport()?;
        node.caller()
            .call_community_envelope(
                &node
                    .import_route(target_route_blob)
                    .map_err(|e| GovernanceRuntimeError::Adapter(format!("import route: {e}")))?,
                payload,
            )
            .await
            .map_err(|e| GovernanceRuntimeError::Adapter(format!("app_call: {e}")))
    }

    /// Re-run the pure CRDT merge. Same function every peer runs, which
    /// is what makes the result convergent.
    pub(super) fn rebuild_governance_state_impl(
        entries: &[(PseudonymKey, Vec<GovernanceEntry>)],
    ) -> GovernanceState {
        rekindle_governance::merge::merge(entries)
    }

    // ---------- Hydration ----------

    pub(super) fn list_communities_for_dht_open_impl(&self) -> Vec<CommunityDhtOpenSetup> {
        let guard = self.ctx.session.read();
        let Some(session) = guard.as_ref() else {
            return Vec::new();
        };
        session
            .communities
            .values()
            .map(|m| {
                // Under `o_cnt: 0` there is no registry owner; a member
                // writes the registry and its channel slots with its derived
                // slot keypair.
                let slot_writer = m.slot_seed.as_ref().and_then(|seed| {
                    rekindle_transport::broadcast::dht_writes::derive_slot_keypair_str(
                        seed,
                        m.slot_index,
                    )
                    .ok()
                });
                CommunityDhtOpenSetup {
                    id: m.governance_key.clone(),
                    governance_key: m.governance_key.clone(),
                    registry_key: (!m.registry_key.is_empty()).then(|| m.registry_key.clone()),
                    registry_writer: slot_writer.clone(),
                    slot_writer,
                }
            })
            .collect()
    }

    /// Invite records this identity published and must keep warm.
    ///
    /// The daemon does not yet persist an invite-secrets inventory —
    /// `InviteCreate` publishes the DFLT record and discards the owner
    /// keypair. Returning empty means our own invites are not
    /// republished by this track and will decay from the DHT when no
    /// reader refreshes them; recorded as a gap rather than papered over.
    pub(super) fn list_my_active_invite_secret_keys_impl() -> Vec<String> {
        Vec::new()
    }

    /// Raise the Lamport counter and install the merged state.
    pub(super) fn apply_governance_rebuild_result_impl(
        &self,
        community_id: &str,
        gov_state: GovernanceState,
        accepted_clock: u64,
    ) {
        // max(), never overwrite: our own unsent entries may already have
        // advanced past what the DHT shows.
        crate::host::session::raise_clock(
            &self.ctx.session,
            &self.ctx.db,
            community_id,
            accepted_clock,
        );
        self.set_governance_state_impl(community_id, gov_state);
    }

    /// Warm cache of the raw per-author entry sets, so the next start can
    /// re-merge losslessly without waiting on the DHT rebuild: the same
    /// `governance_entries_cache` rows the desktop keeps. Queued; the DHT
    /// stays authoritative.
    pub(super) fn persist_governance_entries_cache_impl(
        &self,
        community_id: &str,
        entries: &[(PseudonymKey, Vec<GovernanceEntry>)],
    ) {
        let Some(owner) = self
            .ctx
            .session
            .read()
            .as_ref()
            .map(|s| s.identity.public_key_hex.clone())
        else {
            return;
        };
        let Ok(db) = self.ctx.db.current() else {
            return;
        };
        let (community, entries) = (community_id.to_string(), entries.to_vec());
        let now = rekindle_utils::timestamp_ms_i64();
        db.fire("governance entries cache", move |conn| {
            rekindle_db::repo::governance_cache::save(conn, &owner, &community, &entries, now)
        });
    }

    pub(super) fn persist_discovered_registry_members_impl(
        community_id: &str,
        members: &[DiscoveredMember],
    ) {
        // The daemon keeps no member mirror (see roles.rs): membership
        // is read from the registry and the CRDT on demand. What matters
        // locally is recovering OUR row, which the orchestrator does via
        // apply_recovered_member_state.
        tracing::debug!(
            community_id,
            discovered = members.len(),
            "governance adapter: registry members discovered"
        );
    }

    // ---------- Background tasks ----------
    //
    // The Tauri host spawns per-community loops here. On the daemon the
    // `SubscriptionManager` runs the watch/poll tiers for every joined
    // community, so a second set would double the DHT traffic against the
    // same records. The record keepalive (`RecordPool::rehydrate`) has no
    // daemon equivalent yet: plan item C7.8b. Traced so the coverage
    // decision is visible.

    pub(super) fn spawn_history_catchup_impl(community_id: &str) {
        tracing::debug!(community_id, "history catchup: covered by SMPL catchup");
    }

    /// Queue the forward-secrecy rotation that a ban requires.
    ///
    /// A banned member still holds the current MEK, so the ban is only
    /// half of the removal until the key changes. This used to trace a
    /// line and return, which meant the daemon track never completed
    /// that half.
    ///
    /// Fire-and-forget by contract: the ban entry has already been
    /// written and must not be undone by a rotation failure, so a full
    /// queue or a stopped worker is logged rather than propagated.
    pub(super) fn spawn_text_mek_rotation_for_ban_impl(
        &self,
        community_id: &str,
        banned_pseudonym_hex: &str,
    ) {
        let request = crate::daemon::mek_rotation::MekRotationRequest::departure(
            community_id,
            banned_pseudonym_hex,
        );
        if self.ctx.mek_rotation_tx.send(request).is_err() {
            tracing::warn!(
                community_id,
                "MEK rotation worker is gone — ban did not rotate the key"
            );
        }
    }
}
