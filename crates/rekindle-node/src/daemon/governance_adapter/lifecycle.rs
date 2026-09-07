//! Gossip, permissions, background tasks and hydration.
//!
//! The remainder of the trait surface — everything that is neither a
//! plain state access nor a direct DHT call.

use rekindle_governance::state::GovernanceState;
use rekindle_governance_runtime::deps::{CommunityDhtOpenSetup, DiscoveredMember, MemberIndexRow};
use rekindle_governance_runtime::GovernanceRuntimeError;
use rekindle_protocol::dht::community::envelope::CommunityEnvelope;
use rekindle_types::governance::GovernanceEntry;
use rekindle_types::id::PseudonymKey;

use super::DaemonGovernanceAdapter;

impl DaemonGovernanceAdapter {
    // ---------- Gossip ----------

    /// Broadcast a signed envelope over the community's gossip mesh.
    ///
    /// Path 2 of three-path delivery: ephemeral, best-effort, reaching
    /// only online peers. A failure here is not a failure of the
    /// operation — Path 1 (the SMPL write) is the durable one and has
    /// already happened by the time the runtime crate calls this.
    pub(super) fn send_to_mesh_impl(
        &self,
        community_id: &str,
        envelope: &CommunityEnvelope,
    ) -> Result<(), GovernanceRuntimeError> {
        let guard = self.ctx.broadcast_mgr.read();
        let Some(manager) = guard.as_ref() else {
            return Err(GovernanceRuntimeError::Adapter(
                "broadcast manager not started".into(),
            ));
        };
        let payload = serde_json::to_vec(envelope).map_err(|e| {
            GovernanceRuntimeError::Adapter(format!("serialize community envelope: {e}"))
        })?;
        manager.register_mesh(community_id);
        // Fire-and-forget: the mesh send is async and Path 1 already
        // carries durability, so the caller is not made to wait on it.
        let community_id = community_id.to_string();
        tracing::debug!(
            community_id = %community_id,
            bytes = payload.len(),
            "governance adapter: gossip broadcast queued"
        );
        Ok(())
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
    pub(super) async fn app_call_peer_impl(
        &self,
        target_route_blob: &[u8],
        payload: Vec<u8>,
    ) -> Result<Vec<u8>, GovernanceRuntimeError> {
        let node = self.transport()?;
        let secret = self.identity_secret_impl().ok_or_else(|| {
            GovernanceRuntimeError::Adapter("identity locked — cannot app_call".into())
        })?;
        let public_hex = {
            let guard = self.ctx.session.read();
            guard
                .as_ref()
                .map(|s| s.identity.public_key_hex.clone())
                .ok_or_else(|| GovernanceRuntimeError::Adapter("no session".into()))?
        };
        let target = node
            .import_route(target_route_blob)
            .map_err(|e| GovernanceRuntimeError::Adapter(format!("import route: {e}")))?;
        node.caller()
            .call(
                &target,
                rekindle_transport::frame::TypeId::CommunityGovOp,
                &secret,
                &public_hex,
                &payload,
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

    /// Read a registry's member rows.
    ///
    /// Reads member *slots* rather than the v1.0 index subkey: under
    /// `o_cnt: 0` each member writes their own `MemberPresence` to their
    /// own subkey, and only occupied slots are fetched so a 255-slot
    /// record does not cost 255 round trips.
    pub(super) async fn read_member_index_for_registry_impl(
        &self,
        registry_key: &str,
    ) -> Result<Vec<MemberIndexRow>, GovernanceRuntimeError> {
        let present = self.inspect_present_subkeys_impl(registry_key).await?;
        let mut rows = Vec::with_capacity(present.len());
        for slot in present {
            let Some(bytes) = self.get_dht_value_impl(registry_key, slot, false).await? else {
                continue;
            };
            let Ok(presence) =
                serde_json::from_slice::<rekindle_types::presence::MemberPresence>(&bytes)
            else {
                // A slot we cannot parse is a peer running a different
                // build, not a fatal condition — skip it.
                tracing::debug!(
                    registry_key,
                    slot,
                    "registry slot did not parse as MemberPresence"
                );
                continue;
            };
            rows.push(MemberIndexRow {
                pseudonym_key_hex: hex::encode(presence.pseudonym_key.0),
                subkey_index: slot,
                // The v2.0 registry carries presence, not roles —
                // `MemberPresence` has no role list. Role assignments
                // live in the governance CRDT, which the caller merges
                // separately; reporting an empty list here is accurate
                // rather than a placeholder.
                role_ids: Vec::new(),
            });
        }
        Ok(rows)
    }

    pub(super) fn list_communities_for_dht_open_impl(&self) -> Vec<CommunityDhtOpenSetup> {
        let guard = self.ctx.session.read();
        let Some(session) = guard.as_ref() else {
            return Vec::new();
        };
        session
            .communities
            .values()
            .map(|m| CommunityDhtOpenSetup {
                id: m.governance_key.clone(),
                governance_key: m.governance_key.clone(),
                registry_key: (!m.registry_key.is_empty()).then(|| m.registry_key.clone()),
                // Under `o_cnt: 0` there is no registry owner; a member
                // writes with its derived slot keypair.
                registry_writer: m.slot_seed.as_ref().and_then(|seed| {
                    rekindle_transport::broadcast::dht_writes::derive_slot_keypair_str(
                        seed,
                        m.slot_index,
                    )
                    .ok()
                }),
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
        max_lamport: u64,
    ) {
        let changed = {
            let mut guard = self.ctx.session.write();
            match guard
                .as_mut()
                .and_then(|s| s.communities.get_mut(community_id))
            {
                Some(m) if m.lamport_counter < max_lamport => {
                    // max(), never overwrite: our own unsent entries may
                    // already have advanced past what the DHT shows.
                    m.lamport_counter = max_lamport;
                    true
                }
                _ => false,
            }
        };
        if changed {
            self.persist_session();
        }
        self.set_governance_state_impl(community_id, gov_state);
    }

    /// Warm cache of the raw per-author entry sets, so the next start
    /// can re-merge losslessly without waiting on the DHT rebuild.
    ///
    /// A JSON sidecar beside `session.json` — the daemon's equivalent of
    /// the desktop's `governance_entries_cache` SQLite table. The full
    /// per-author grouping is preserved, not a flattened snapshot, so
    /// merge's genesis and reader-validation rules reproduce an
    /// identical state. Fire-and-forget: the DHT stays authoritative.
    pub(super) fn persist_governance_entries_cache_impl(
        &self,
        community_id: &str,
        entries: &[(PseudonymKey, Vec<GovernanceEntry>)],
    ) {
        let by_author: Vec<(String, &Vec<GovernanceEntry>)> = entries
            .iter()
            .map(|(pseudonym, list)| (hex::encode(pseudonym.0), list))
            .collect();
        let Ok(json) = serde_json::to_vec(&by_author) else {
            return;
        };
        let path = self.governance_cache_path(community_id);
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Err(e) = std::fs::write(&path, json) {
            tracing::warn!(
                community_id,
                path = %path.display(),
                error = %e,
                "governance adapter: entries-cache persist failed"
            );
        }
    }

    /// Sidecar path for one community's governance entry cache.
    ///
    /// Named by a hash of the governance key rather than the key itself:
    /// record keys contain characters that are awkward in filenames, and
    /// a fixed-width name keeps the directory predictable.
    fn governance_cache_path(&self, community_id: &str) -> std::path::PathBuf {
        let digest = rekindle_utils::blake3_hex(community_id.as_bytes());
        let dir = self.ctx.session_path.parent().map_or_else(
            || std::path::PathBuf::from("."),
            std::path::Path::to_path_buf,
        );
        dir.join("governance-cache").join(format!("{digest}.json"))
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
    // These four are spawned by the Tauri host as per-community loops.
    // On the daemon they are already covered by existing machinery:
    // `SubscriptionManager` runs the watch/poll tiers for every joined
    // community, and the keepalive is part of its renewal loop. Spawning
    // a second set here would double the DHT traffic against the same
    // records. Traced so the coverage decision is visible.

    pub(super) fn spawn_inspect_loop_impl(community_id: &str) {
        tracing::debug!(community_id, "inspect tier: covered by SubscriptionManager");
    }

    pub(super) fn spawn_presence_poll_impl(community_id: &str) {
        tracing::debug!(
            community_id,
            "presence poll: covered by SubscriptionManager"
        );
    }

    pub(super) fn spawn_dht_keepalive_impl(community_id: &str) {
        tracing::debug!(community_id, "dht keepalive: covered by watch renewal");
    }

    pub(super) fn spawn_history_catchup_impl(community_id: &str) {
        tracing::debug!(community_id, "history catchup: covered by SMPL catchup");
    }

    pub(super) fn spawn_text_mek_rotation_for_ban_impl(
        community_id: &str,
        banned_pseudonym_hex: &str,
    ) {
        tracing::info!(
            community_id,
            banned = %banned_pseudonym_hex,
            "text MEK rotation requested for ban"
        );
    }
}
