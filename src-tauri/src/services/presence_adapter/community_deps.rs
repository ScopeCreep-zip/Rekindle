//! Phase 21 REDO — `CommunityPresenceDeps` impl for `PresenceAdapter`.
//!
//! Owns the 21-method community-presence surface: per-community
//! state reads (pseudonym key, route blob, channel list), DHT
//! orchestration (registry write, channel-log catch-up read), DB
//! persistence (history range computation, member upsert), MEK
//! encrypt + W26 signature, and community event emission.

use std::collections::{HashMap, HashSet};

use async_trait::async_trait;
use rekindle_codec::community::envelope::{CommunityEnvelope, SignedEnvelope};
use rekindle_presence::{
    CommunityPresenceDeps, DiscoveredMemberRow, GossipOverlayPlan, GossipOverlaySnapshot,
    OnlineMember, PresenceCredentials, PresenceError, SegmentDescriptor, SelfPresenceSnapshot,
};

use crate::services::presence_adapter::PresenceAdapter;
use crate::state_helpers;

#[async_trait]
impl CommunityPresenceDeps for PresenceAdapter {
    fn now_secs(&self) -> u64 {
        rekindle_utils::timestamp_secs()
    }

    fn rng(&self) -> &dyn rekindle_utils::RandomSource {
        &rekindle_utils::OsRandom
    }

    fn scope(&self) -> std::sync::Arc<rekindle_lifecycle::SessionScope> {
        state_helpers::login_scope_or_closed(&self.state)
    }

    fn my_pseudonym_for_community(&self, community_id: &str) -> String {
        super::state_reads::my_pseudonym_for_community(&self.state, community_id)
    }

    fn our_route_blob(&self) -> Option<Vec<u8>> {
        state_helpers::our_route_blob(&self.state)
    }

    async fn own_route_available(&self) {
        if let Some(routes) = state_helpers::own_routes(&self.state) {
            routes
                .available(rekindle_records::port::RouteClass::General)
                .await;
        }
    }

    fn current_presence_status_str(&self, community_id: &str) -> String {
        // Delegate to the src-tauri helper so the wire-string
        // mapping lives in exactly one place. The helper is the
        // wire-up point for per-community `MemberPresence.custom_status`
        // (architecture spec line 754); a future commit can resolve
        // `community.my_custom_status` first inside the helper and
        // the adapter automatically picks it up.
        crate::services::community::current_presence_status(&self.state, community_id).to_string()
    }

    fn send_to_mesh(&self, community_id: &str, envelope: CommunityEnvelope) {
        if let Err(error) = crate::state_helpers::host_governance(&self.state)
            .map(|host| host.send_to_mesh_impl(community_id, &envelope))
            .map_err(|e| e.to_string())
        {
            tracing::debug!(
                community = %community_id,
                %error,
                "send_to_mesh from community-presence deps failed",
            );
        }
    }

    fn mark_initial_sync_done(&self, community_id: &str) {
        super::state_reads::mark_initial_sync_done(&self.state, community_id);
    }

    fn self_presence_snapshot(&self, community_id: &str) -> SelfPresenceSnapshot {
        let communities = self.state.communities.read();
        let Some(community) = communities.get(community_id) else {
            return SelfPresenceSnapshot::default();
        };
        SelfPresenceSnapshot {
            bio: community.my_bio.clone(),
            pronouns: community.my_pronouns.clone(),
            theme_color: community.my_theme_color,
            badges: community.my_badges.clone(),
            avatar_ref: community.my_avatar_ref.clone(),
            banner_ref: community.my_banner_ref.clone(),
        }
    }

    fn encrypt_history_ranges_with_current_mek(
        &self,
        community_id: &str,
        ranges: &[rekindle_types::presence::HistoryRange],
    ) -> Option<rekindle_types::presence::EncryptedHistoryRanges> {
        let mek = state_helpers::current_mek(
            &self.state,
            community_id,
            rekindle_types::channel_keys::KeyScope::Community,
        )?;
        let plaintext = serde_json::to_vec(ranges).ok()?;
        let ciphertext = mek.encrypt(&plaintext).ok()?;
        Some(rekindle_types::presence::EncryptedHistoryRanges {
            mek_generation: mek.generation(),
            ciphertext,
        })
    }

    fn self_session(&self, community_id: &str) -> rekindle_types::presence::MemberSession {
        // The identity's status is the session vocabulary (one language
        // for friends and community presence).
        let status = state_helpers::identity_status(&self.state).unwrap_or_default();
        let location = {
            let communities = self.state.communities.read();
            communities
                .get(community_id)
                .and_then(|c| c.my_session_location.clone())
        };
        rekindle_types::presence::MemberSession {
            status,
            location,
            // Activity (game string) is policy-gated and wired in a
            // follow-up; the session always carries a fresh last_active
            // so peers can bucket our last-seen.
            activity: None,
            last_active: rekindle_utils::timestamp_secs(),
        }
    }

    fn presence_policy(
        &self,
        community_id: &str,
    ) -> rekindle_types::presence::PresenceSharingPolicy {
        let communities = self.state.communities.read();
        communities
            .get(community_id)
            .map(|c| c.presence_policy.clone())
            .unwrap_or_default()
    }

    fn encrypt_session_extras_with_current_mek(
        &self,
        community_id: &str,
        extras: &rekindle_types::presence::SessionExtras,
    ) -> Option<rekindle_types::presence::EncryptedSessionExtras> {
        let mek = state_helpers::current_mek(
            &self.state,
            community_id,
            rekindle_types::channel_keys::KeyScope::Community,
        )?;
        let plaintext = serde_json::to_vec(extras).ok()?;
        let ciphertext = mek.encrypt(&plaintext).ok()?;
        Some(rekindle_types::presence::EncryptedSessionExtras {
            mek_generation: mek.generation(),
            ciphertext,
        })
    }

    fn decrypt_session_extras(
        &self,
        community_id: &str,
        encrypted: &rekindle_types::presence::EncryptedSessionExtras,
    ) -> Option<rekindle_types::presence::SessionExtras> {
        // Generation must match — a rotated-out ex-member's extras are
        // unreadable, which is the intended MEK-bounded readership.
        let mek = state_helpers::current_mek(
            &self.state,
            community_id,
            rekindle_types::channel_keys::KeyScope::Community,
        )
        .filter(|mek| mek.generation() == encrypted.mek_generation)?;
        let plaintext = mek.decrypt(&encrypted.ciphertext).ok()?;
        serde_json::from_slice(&plaintext).ok()
    }

    async fn compute_history_ranges(
        &self,
        community_id: &str,
    ) -> Vec<rekindle_types::presence::HistoryRange> {
        super::persist::compute_history_ranges(&self.state, &self.pool, community_id).await
    }

    fn sign_presence_row(&self, community_id: &str, signing_bytes: &[u8]) -> Option<Vec<u8>> {
        let (_, signing_key) =
            state_helpers::pseudonym_credentials(&self.state, community_id).ok()?;
        let sig = rekindle_secrets::derive::sign_with_pseudonym(&signing_key, signing_bytes);
        Some(sig.to_vec())
    }

    async fn write_presence_to_registry_subkey(
        &self,
        registry_key: &str,
        subkey_index: u32,
        presence_json: Vec<u8>,
        writer_keypair_str: &str,
    ) -> Result<rekindle_presence::RowWrite, PresenceError> {
        let outcome = self
            .record_pool()?
            .write_once(
                registry_key,
                subkey_index,
                presence_json,
                Some(writer_keypair_str),
            )
            .await
            .map_err(super::presence_error)?;
        // A presence row asserts the present: a miss is an error, never a
        // write queued to land later. A supersede goes back to the writer,
        // which decides from whose row it is (plan C7.16).
        use rekindle_records::port::SetOutcome;
        match outcome {
            SetOutcome::Landed | SetOutcome::Unchanged => Ok(rekindle_presence::RowWrite::Stored),
            SetOutcome::Superseded(newer) => Ok(rekindle_presence::RowWrite::Superseded {
                seq: newer.seq,
                data: newer.data,
            }),
            missed @ SetOutcome::Offline => Err(PresenceError::Dht(format!(
                "presence not stored ({missed:?})"
            ))),
        }
    }

    fn persist_discovered_member_rows(
        &self,
        community_id: &str,
        rows: Vec<DiscoveredMemberRow>,
        banned_pseudonyms: Vec<String>,
        joined_at: i64,
    ) {
        super::persist::upsert_discovered_member_rows(
            &self.state,
            &self.pool,
            community_id,
            rows,
            banned_pseudonyms,
            joined_at,
        );
    }

    fn extend_known_members(&self, community_id: &str, candidates: Vec<String>) -> Vec<String> {
        let mut communities = self.state.communities.write();
        let Some(cs) = communities.get_mut(community_id) else {
            return Vec::new();
        };
        candidates
            .into_iter()
            .filter(|key| cs.known_members.insert(key.clone()))
            .collect()
    }

    fn emit_member_discovered(&self, community_id: &str, pseudonym_key: &str, subkey_index: u32) {
        let key = rekindle_types::id::PseudonymKey::from_hex_lossy(pseudonym_key);
        crate::event_dispatch::emit_membership(
            &self.app_handle,
            rekindle_types::subscription_events::MembershipEvent::MemberDiscovered {
                community: community_id.to_string(),
                pseudonym: pseudonym_key.to_string(),
                // No presence row names a member (plan E3.4 S6): the short
                // pseudonym fingerprint until its verified card is read.
                display_name: rekindle_crypto::identity::short_key_fingerprint(&key.0),
                subkey_index,
            },
        );
    }

    async fn run_presence_poll_tick(&self, community_id: &str) -> Result<(), String> {
        // The cadence loop in `spawn.rs` invokes this from each timer
        // tick; the tick is left at stop (record-pool work only).
        crate::services::community::presence_poll_tick_public(&self.state, community_id).await
    }

    // -- presence_poll_tick surface (21.i-REDO) --

    async fn ensure_registry_open(&self, community_id: &str) -> Result<Option<String>, String> {
        super::state_reads::ensure_registry_open(&self.state, community_id).await
    }

    fn presence_credentials(&self, community_id: &str) -> Option<PresenceCredentials> {
        super::state_reads::presence_credentials(&self.state, community_id)
    }

    fn governance_bans(&self, community_id: &str) -> HashSet<String> {
        super::state_reads::governance_bans(&self.state, community_id)
    }

    fn segment_descriptors(&self, community_id: &str) -> Vec<SegmentDescriptor> {
        super::state_reads::segment_descriptors(&self.state, community_id)
    }

    async fn scan_segment_raw(
        &self,
        registry_key: &str,
        max_subkey: u32,
        skip_subkey: Option<u32>,
    ) -> Vec<(u32, Vec<u8>)> {
        super::scan::scan_segment_raw(&self.state, registry_key, max_subkey, skip_subkey).await
    }

    fn apply_member_state_update(
        &self,
        community_id: &str,
        known_member_keys: HashSet<String>,
        banned_members: &HashSet<String>,
    ) {
        super::member_state::apply_member_state_update(
            &self.state,
            community_id,
            known_member_keys,
            banned_members,
        );
    }

    fn read_member_profile_snapshot(
        &self,
        community_id: &str,
    ) -> HashMap<String, rekindle_presence::MemberProfileSnapshot> {
        super::member_state::read_member_profile_snapshot(&self.state, community_id)
    }

    fn apply_member_profile_updates(
        &self,
        community_id: &str,
        updates: HashMap<String, rekindle_presence::MemberProfileSnapshot>,
        emit_refreshed: bool,
    ) {
        super::member_state::apply_member_profile_updates(
            &self.state,
            &self.app_handle,
            community_id,
            updates,
            emit_refreshed,
        );
    }

    fn extend_online_with_recent_gossip(
        &self,
        community_id: &str,
        online_members: &mut HashMap<String, OnlineMember>,
        my_pseudonym: &str,
        eviction_threshold_secs: u64,
    ) {
        super::gossip_overlay::extend_online_with_recent_gossip(
            &self.state,
            community_id,
            online_members,
            my_pseudonym,
            eviction_threshold_secs,
        );
    }

    fn gossip_offline_diff(
        &self,
        community_id: &str,
        online_members: &HashMap<String, OnlineMember>,
        my_pseudonym: &str,
    ) -> Vec<String> {
        super::gossip_overlay::gossip_offline_diff(
            &self.state,
            community_id,
            online_members,
            my_pseudonym,
        )
    }

    fn read_gossip_snapshot(&self, community_id: &str) -> GossipOverlaySnapshot {
        super::gossip_overlay::read_gossip_snapshot(&self.state, community_id)
    }

    fn apply_gossip_rebuild_plan(&self, community_id: &str, plan: GossipOverlayPlan) {
        super::gossip_overlay::apply_gossip_rebuild_plan(&self.state, community_id, plan);
    }

    fn send_to_mesh_raw(&self, _community_id: &str, envelope: SignedEnvelope) {
        // Held broadcasts go to the Host's mesh unchanged (plan E3.4 S5).
        if let Ok(host) = crate::state_helpers::host_governance(&self.state) {
            host.forward_signed(envelope);
        }
    }

    fn emit_member_presence_offline(&self, community_id: &str, pseudonym_key: &str) {
        super::gossip_overlay::emit_member_presence_offline(
            &self.state,
            community_id,
            pseudonym_key,
        );
    }

    fn active_voice_channel(&self, community_id: &str) -> Option<String> {
        let ve = self.state.voice_engine.lock();
        ve.as_ref()
            .filter(|h| h.community_id.as_deref() == Some(community_id))
            .map(|h| h.channel_id.clone())
    }

    fn reconcile_voice_roster(
        &self,
        community_id: &str,
        rows: Vec<rekindle_presence::VoicePresenceRow>,
    ) {
        // Cheap pre-gate before building the signaling deps: skip
        // entirely when no voice session is bound to this community.
        if self.active_voice_channel(community_id).is_none() {
            return;
        }
        let Some(app) = self.state.app_handle.read().clone() else {
            return;
        };
        let Ok(deps) =
            crate::services::voice_runtime::build_voice_signaling_deps(&app, &self.state)
        else {
            return;
        };
        let views: Vec<rekindle_voice::signaling::PresencePeerView> = rows
            .into_iter()
            .map(|r| rekindle_voice::signaling::PresencePeerView {
                pseudonym_hex: r.pseudonym_hex,
                display_name: r.display_name,
                voice_channel_id: r.voice_channel_id,
                fresh: r.fresh,
            })
            .collect();
        let cid = community_id.to_string();
        crate::state_helpers::login_scope_or_closed(&self.state).spawn_or_drop(
            "voice roster reconcile",
            async move {
                rekindle_voice::signaling::reconcile_from_presence(&deps, &cid, views).await;
            },
        );
    }

    fn maybe_auto_expand_segment(&self, community_id: &str) {
        super::auto_expand::maybe_auto_expand_segment(&self.state, community_id);
    }
}
