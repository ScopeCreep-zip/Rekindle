//! Phase 14.k — voice signaling adapter.
//!
//! Implements `rekindle_voice::signaling::VoiceSignalingDeps` against
//! the live `AppState` + `tauri::AppHandle` + `Db` + the existing
//! `services::community::*` cross-subsystem functions (which Phase 17 /
//! 19 / 20 will eventually own). The crate's signaling handlers
//! (voice_join / voice_leave / stage_update / etc.) consume this trait
//! via `Arc<dyn VoiceSignalingDeps>`.

use std::sync::Arc;

use async_trait::async_trait;
use rekindle_codec::community::envelope::CommunityEnvelope;
use rekindle_voice::signaling::{CommunityVoiceEvent, StageChannelInfo, VoiceSignalingDeps};
use rekindle_voice::transport::VoiceTransport;
use tokio::sync::Mutex as AsyncMutex;

use crate::state::{AppState, ChannelType};
use crate::state_helpers;

mod events;
use rekindle_db::Db;

pub struct VoiceSignalingAdapter {
    state: Arc<AppState>,
    app_handle: tauri::AppHandle,
}

impl VoiceSignalingAdapter {
    /// `_pool` accepted to match the construction shape of the other
    /// Phase-14 adapters (calls, voice, dm) even though signaling
    /// doesn't reach SQLite directly — `persist_hand_raise` goes
    /// through `services::community::persist_hand_raise(&AppState, ...)`.
    #[must_use]
    pub fn new(state: Arc<AppState>, app_handle: tauri::AppHandle, _pool: Db) -> Arc<Self> {
        Arc::new(Self { state, app_handle })
    }
}

/// Public free-fn facade — dispatch an inbound community voice
/// gossip `ControlPayload` through the crate handler. Used by
/// `services::veilid::control_moderation` (the gossip dispatcher).
/// Spawns the handler so the caller stays sync.
pub fn handle_voice_signaling(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    community_id: &str,
    sender_pseudonym: &str,
    payload: rekindle_codec::community::envelope::ControlPayload,
) {
    let Ok(pool) = state.db.current() else {
        tracing::debug!("voice signaling: no identity database — dropped");
        return;
    };
    let adapter = VoiceSignalingAdapter::new(state.clone(), app_handle.clone(), pool);
    let deps: Arc<dyn VoiceSignalingDeps> = adapter;
    let cid = community_id.to_string();
    let sender = sender_pseudonym.to_string();
    crate::state_helpers::login_scope_or_closed(state).spawn_or_drop(
        "voice signaling",
        async move {
            rekindle_voice::signaling::handle_voice_signaling(deps, &cid, &sender, payload).await;
        },
    );
}

/// `departed` left the community: drop them from the voice channel we
/// share and rotate our media key
/// (`rekindle_voice::signaling::member_departed`, plan C7.20).
pub fn member_departed(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    community_id: &str,
    departed: &str,
) {
    let Ok(pool) = state.db.current() else {
        return;
    };
    let deps: Arc<dyn VoiceSignalingDeps> =
        VoiceSignalingAdapter::new(state.clone(), app_handle.clone(), pool);
    rekindle_voice::signaling::member_departed(&deps, community_id, departed);
}

#[async_trait]
impl VoiceSignalingDeps for VoiceSignalingAdapter {
    fn my_pseudonym(&self, community_id: &str) -> Option<String> {
        state_helpers::my_pseudonym_key(&self.state, community_id)
    }

    fn our_media_route_blob(&self) -> Option<Vec<u8>> {
        // The blob peers import to send us voice and video: the half of
        // the path `frame_sender`'s LowLatency + PreferUnordered context
        // cannot reach on its own. Same route the session path
        // advertises; never the general route (plan C7.9c).
        state_helpers::our_media_route_blob(&self.state)
    }

    fn stage_channel_info(&self, community_id: &str, channel_id: &str) -> Option<StageChannelInfo> {
        let communities = self.state.communities.read();
        let community = communities.get(community_id)?;
        let channel = community.channels.iter().find(|ch| ch.id == channel_id)?;
        Some(StageChannelInfo {
            is_stage: matches!(channel.channel_type, ChannelType::Stage),
            speakers: channel.stage_speakers.clone(),
            moderator: channel.stage_moderator.clone(),
        })
    }

    fn update_stage_channel(
        &self,
        community_id: &str,
        channel_id: &str,
        topic: Option<String>,
        speakers: Vec<String>,
        moderator: String,
    ) {
        let mut communities = self.state.communities.write();
        if let Some(community) = communities.get_mut(community_id) {
            if let Some(channel) = community.channels.iter_mut().find(|ch| ch.id == channel_id) {
                if let Some(t) = topic {
                    channel.topic = t;
                }
                channel.stage_speakers = speakers;
                channel.stage_moderator = Some(moderator);
            }
        }
    }

    fn online_voice_members(&self, community_id: &str) -> Vec<(String, Vec<u8>)> {
        let communities = self.state.communities.read();
        let Some(community) = communities.get(community_id) else {
            return Vec::new();
        };
        let Some(gossip) = community.gossip.as_ref() else {
            return Vec::new();
        };
        gossip
            .online_members
            .iter()
            .map(|(pk, m)| (pk.clone(), m.route_blob.clone()))
            .collect()
    }

    fn decode_channel_id(&self, channel_id: &str) -> Option<[u8; 16]> {
        hex::decode(channel_id).ok()?.try_into().ok()
    }

    fn my_permissions(&self, community_id: &str, channel_id: Option<[u8; 16]>) -> u64 {
        let ch_id = channel_id.map(rekindle_types::id::ChannelId);
        state_helpers::my_permissions(&self.state, community_id, ch_id.as_ref())
    }

    fn sender_has_perm(
        &self,
        community_id: &str,
        sender_pseudonym_hex: &str,
        perm_mask: u64,
    ) -> bool {
        use rekindle_governance::permissions::{compute_permissions, has_all_capabilities};
        use rekindle_types::id::PseudonymKey;

        let communities = self.state.communities.read();
        let Some(community) = communities.get(community_id) else {
            return false;
        };
        let Some(gov) = community.governance_state.as_ref() else {
            return false;
        };
        let Ok(bytes) = hex::decode(sender_pseudonym_hex) else {
            return false;
        };
        let Ok(arr) = <[u8; 32]>::try_from(bytes.as_slice()) else {
            return false;
        };
        let perms = compute_permissions(
            &PseudonymKey(arr),
            None,
            gov,
            rekindle_utils::timestamp_secs(),
        );
        has_all_capabilities(perms, perm_mask)
    }

    fn transport_handle(&self) -> Option<Arc<AsyncMutex<VoiceTransport>>> {
        self.state
            .voice_engine
            .lock()
            .as_ref()
            .map(|h| h.transport.clone())
    }

    fn voice_engine_channel_id(&self) -> Option<String> {
        self.state
            .voice_engine
            .lock()
            .as_ref()
            .map(|h| h.channel_id.clone())
    }

    fn voice_engine_bound_to(&self, community_id: &str, channel_id: &str) -> bool {
        let ve = self.state.voice_engine.lock();
        ve.as_ref().is_some_and(|h| {
            h.community_id.as_deref() == Some(community_id) && h.channel_id == channel_id
        })
    }

    fn set_voice_engine_muted(&self, muted: bool) {
        crate::state_helpers::set_voice_engine_muted(&self.state, muted);
    }

    fn set_voice_engine_deafened(&self, deafened: bool) {
        crate::state_helpers::set_voice_engine_deafened(&self.state, deafened);
    }

    fn media_live_peers(&self) -> std::collections::HashSet<String> {
        state_helpers::media_live_peers(&self.state)
    }

    fn channel_sender_keys(
        &self,
        community_id: &str,
        channel_id: &str,
    ) -> Arc<rekindle_secrets::media_sender_key::keyring::ChannelSenderKeys> {
        self.state.voice_sender_keys.keys(community_id, channel_id)
    }

    fn seal_media_key(
        &self,
        community_id: &str,
        recipient: &str,
        aad: &[u8],
        secret: &[u8; 32],
    ) -> Option<Vec<u8>> {
        let (_, signing_key) =
            state_helpers::pseudonym_credentials(&self.state, community_id).ok()?;
        let recipient: [u8; 32] = hex::decode(recipient).ok()?.try_into().ok()?;
        rekindle_secrets::media_sender_key::seal(&signing_key, &recipient, aad, secret).ok()
    }

    fn open_media_key(
        &self,
        community_id: &str,
        sender: &str,
        aad: &[u8],
        sealed: &[u8],
    ) -> Option<zeroize::Zeroizing<[u8; 32]>> {
        let (_, signing_key) =
            state_helpers::pseudonym_credentials(&self.state, community_id).ok()?;
        let sender: [u8; 32] = hex::decode(sender).ok()?.try_into().ok()?;
        rekindle_secrets::media_sender_key::open(&signing_key, &sender, aad, sealed).ok()
    }

    fn send_to_mesh(&self, community_id: &str, envelope: &CommunityEnvelope) {
        if let Err(e) =
            crate::services::community::send_to_mesh(&self.state, community_id, envelope)
        {
            tracing::debug!(community = %community_id, error = %e, "voice send_to_mesh failed");
        }
    }

    fn advertise_media_capabilities(&self, community_id: &str, channel_id: &str) {
        if let Err(e) = crate::services::voice_adapter::advertise_media_capabilities(
            &self.state,
            community_id,
            channel_id,
        ) {
            tracing::debug!(
                community = %community_id,
                channel = %channel_id,
                error = %e,
                "directed media-caps advertise skipped"
            );
        }
    }

    async fn persist_hand_raise(&self, community_id: String, channel_id: String, raised: bool) {
        if let Err(error) = crate::services::community::persist_hand_raise(
            &self.state,
            &community_id,
            &channel_id,
            raised,
        )
        .await
        {
            tracing::debug!(
                community = %community_id,
                channel = %channel_id,
                error = %error,
                "persist hand raise failed"
            );
        }
    }

    fn next_lamport(
        &self,
        community_id: &str,
    ) -> Result<u64, rekindle_types::lamport::LamportError> {
        state_helpers::next_message_lamport(&self.state, community_id)
    }

    fn stage_speakers(&self, community_id: &str, channel_id: &str) -> Vec<String> {
        let communities = self.state.communities.read();
        communities
            .get(community_id)
            .and_then(|community| {
                community
                    .channels
                    .iter()
                    .find(|channel| channel.id == channel_id)
                    .map(|channel| channel.stage_speakers.clone())
            })
            .unwrap_or_default()
    }

    fn start_mcu_loop(&self) {
        // Build a fresh VoiceAdapter (Arc<dyn VoiceSessionDeps>) and
        // delegate to the crate's start_mcu_loop. Pool acquisition
        // is best-effort — if it's missing, we log and skip rather
        // than panic (matches the legacy services::voice::session
        // facade behavior).
        let Ok(pool) = self.state.db.current() else {
            tracing::debug!("start_mcu_loop: no identity database");
            return;
        };
        let adapter = crate::services::voice_adapter::VoiceAdapter::new(
            self.state.clone(),
            self.app_handle.clone(),
            pool,
        );
        let deps: Arc<dyn rekindle_voice::VoiceSessionDeps> = adapter;
        if let Err(e) = rekindle_voice::session::start_mcu_loop(&deps) {
            tracing::warn!(error = %e, "start_mcu_loop failed");
        }
    }

    async fn stop_mcu_loop(&self) {
        let Ok(pool) = self.state.db.current() else {
            return;
        };
        let adapter = crate::services::voice_adapter::VoiceAdapter::new(
            self.state.clone(),
            self.app_handle.clone(),
            pool,
        );
        let deps: Arc<dyn rekindle_voice::VoiceSessionDeps> = adapter;
        rekindle_voice::session::stop_mcu_loop(&deps).await;
    }

    fn send_to_channel(
        &self,
        community_id: &str,
        channel_id: &str,
        envelope: &rekindle_codec::community::envelope::CommunityEnvelope,
    ) {
        if let Err(e) = crate::services::community::send_to_channel_peers(
            &self.state,
            community_id,
            channel_id,
            envelope,
        ) {
            tracing::debug!(
                community = %community_id,
                channel = %channel_id,
                error = %e,
                "voice signaling directed send failed",
            );
        }
    }

    fn my_display_name(&self) -> Option<String> {
        let name = crate::state_helpers::identity_display_name(&self.state);
        (!name.is_empty()).then_some(name)
    }

    fn emit_event(&self, event: CommunityVoiceEvent) {
        events::emit(&self.state, &self.app_handle, event);
    }

    fn scope(&self) -> std::sync::Arc<rekindle_lifecycle::SessionScope> {
        state_helpers::login_scope_or_closed(&self.state)
    }
}
