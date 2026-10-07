//! Phase 14.r split — `impl VoiceSessionDeps for VoiceAdapter`.
//!
//! All voice-session trait surface in one place. Methods either
//! read/mutate AppState directly (under parking_lot or atomics) or
//! delegate into the helper modules (`session_setup`, `event_mapping`,
//! `io_helpers`) for bodies that would otherwise blow the file-size
//! cap.

use rekindle_types::subscription_events::{SubscriptionEvent, VoiceEvent};
use std::sync::atomic::Ordering;
use std::sync::Arc;

use async_trait::async_trait;
use rekindle_voice::{
    AudioPrefs, VoiceError, VoiceIdentity, VoiceLoopScopes, VoicePeer, VoiceSessionDeps,
    VoiceSessionEvent, VoiceSessionStartup, VoiceShutdownOpts,
};

use super::{event_mapping, io_helpers, session_setup, VoiceAdapter};
use crate::db_helpers::db_call_or_default;
use crate::state_helpers;

#[async_trait]
impl VoiceSessionDeps for VoiceAdapter {
    fn owner_key(&self) -> Result<String, VoiceError> {
        state_helpers::current_owner_key(&self.state).map_err(|_| VoiceError::IdentityNotLoaded)
    }

    fn voice_self_identity(&self, community_id: Option<&str>) -> String {
        state_helpers::voice_self_identity(&self.state, community_id)
    }

    fn identity_secret(&self) -> Result<[u8; 32], VoiceError> {
        state_helpers::identity_secret(&self.state).ok_or(VoiceError::IdentityNotLoaded)
    }

    fn voice_engine_present(&self) -> bool {
        crate::state_helpers::voice_engine_present(&self.state)
    }

    fn set_voice_engine_muted(&self, muted: bool) {
        crate::state_helpers::set_voice_engine_muted(&self.state, muted);
    }

    fn set_voice_engine_deafened(&self, deafened: bool) {
        crate::state_helpers::set_voice_engine_deafened(&self.state, deafened);
    }

    fn pre_stage_voice_channel(&self) {
        let (tx, rx) = tokio::sync::mpsc::channel(200);
        *self.state.voice_packet_tx.write() = Some(tx);
        *self.state.voice_packet_rx_staged.lock() = Some(rx);
    }

    fn clear_voice_channels(&self) {
        *self.state.voice_packet_tx.write() = None;
        *self.state.voice_packet_rx_staged.lock() = None;
        // The report channel goes with them — a sender left behind
        // after the call ends queues reports into a loop that is gone.
        *self.state.voice_report_tx.write() = None;
    }

    fn request_mek_refresh(&self, community_id: &str, channel_id: &str, needed_generation: u64) {
        // Exact-generation request resolved from the undecryptable
        // frame's KID (0 = "send me your current") — never a guess; the
        // responder can always satisfy it, so recovery converges.
        let Some(my_pseudonym) = self
            .state
            .communities
            .read()
            .get(community_id)
            .and_then(|c| c.my_pseudonym_key.clone())
        else {
            return;
        };
        let Some(scope) = crate::state_helpers::media_scope(&self.state, community_id, channel_id)
        else {
            return;
        };
        crate::services::community::mek_rotation::spawn_mek_request_with_retry(
            std::sync::Arc::clone(&self.state),
            community_id.to_string(),
            scope,
            needed_generation,
            my_pseudonym,
        );
    }

    fn send_receiver_report(&self, peer_pubkey_hex: &str, wire: Vec<u8>) {
        io_helpers::send_receiver_report_impl(
            &self.state,
            self.current_shared_transport(),
            peer_pubkey_hex,
            wire,
        );
    }

    fn voice_peers(&self, community_id: &str, _channel_id: &str) -> Vec<VoicePeer> {
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
            .map(|(pseudonym, member)| VoicePeer {
                pseudonym: pseudonym.clone(),
                display_name: String::new(),
                route_blob: Some(member.route_blob.clone()),
            })
            .collect()
    }

    fn channel_is_stage(&self, community_id: &str, channel_id: &str) -> bool {
        state_helpers::channel_is_stage(&self.state, community_id, channel_id)
    }

    fn we_are_stage_speaker(
        &self,
        community_id: &str,
        channel_id: &str,
        our_pseudonym: &str,
    ) -> bool {
        let communities = self.state.communities.read();
        let Some(community) = communities.get(community_id) else {
            return false;
        };
        let Some(channel) = community.channels.iter().find(|ch| ch.id == channel_id) else {
            return false;
        };
        if channel
            .stage_moderator
            .as_deref()
            .is_some_and(|m| m == our_pseudonym)
        {
            return true;
        }
        channel.stage_speakers.iter().any(|s| s == our_pseudonym)
    }

    fn sender_is_stage_speaker(
        &self,
        community_id: &str,
        channel_id: &str,
        sender_pseudonym: &str,
    ) -> bool {
        let communities = self.state.communities.read();
        let Some(community) = communities.get(community_id) else {
            return true;
        };
        let Some(channel) = community.channels.iter().find(|ch| ch.id == channel_id) else {
            return true;
        };
        channel.stage_speakers.iter().any(|s| s == sender_pseudonym)
    }

    fn record_packet_drop(&self) {
        self.state.voice_pkt_drops.fetch_add(1, Ordering::Relaxed);
    }

    fn packet_drops(&self) -> u64 {
        self.state.voice_pkt_drops.load(Ordering::Relaxed)
    }

    fn emit_voice_event(&self, event: VoiceSessionEvent) {
        // Session membership (video-session caps slots + media-ready
        // roster input) is driven by the SIGNALING roster via
        // `CommunityVoiceEvent::VoiceRosterChanged` in the signaling
        // adapter — NOT from these media-driven events. UserJoined
        // fires only on the first received voice packet and UserLeft
        // on silence timeout, so gating membership on them deadlocked
        // video egress behind inbound audio and dropped present-but-
        // quiet peers. These events remain UI-facing only.
        // Phase 5 — quality + receive-stats merge into ONE UI event
        // (each emission carries both halves from the cache).
        // Every event below belongs to the running session, so the
        // scope is read once and shared. No session, nothing to report.
        let Some(scope) = state_helpers::current_voice_scope(&self.state) else {
            return;
        };
        if let Some(merged) = event_mapping::merge_quality_event(&self.state, &event, &scope) {
            crate::event_dispatch::emit_subscription(
                &self.app_handle,
                &SubscriptionEvent::Voice(merged),
            );
            return;
        }
        crate::event_dispatch::emit_subscription(
            &self.app_handle,
            &SubscriptionEvent::Voice(event_mapping::map(event, &scope)),
        );
    }

    fn scope(&self) -> std::sync::Arc<rekindle_lifecycle::SessionScope> {
        state_helpers::login_scope_or_closed(&self.state)
    }

    // ── Phase 14.l — session orchestration deps ─────────────────

    fn current_identity(&self) -> Result<VoiceIdentity, VoiceError> {
        let id = state_helpers::current_identity(&self.state)
            .map_err(|_| VoiceError::IdentityNotLoaded)?;
        Ok(VoiceIdentity {
            public_key: id.public_key,
            display_name: id.display_name,
        })
    }

    fn check_not_in_call(&self, channel_id: &str) -> Result<(), VoiceError> {
        let ve = self.state.voice_engine.lock();
        match ve.as_ref() {
            None => Ok(()),
            Some(handle) if handle.channel_id == channel_id => Ok(()),
            Some(_) => Err(VoiceError::Session(
                "already in a different voice channel".into(),
            )),
        }
    }

    fn audio_prefs(&self) -> AudioPrefs {
        let prefs =
            tauri::Manager::try_state::<crate::commands::settings::Preferences>(&self.app_handle)
                .map(|s| (*s.inner()).clone())
                .unwrap_or_default();
        AudioPrefs {
            noise_suppression: prefs.noise_suppression,
            echo_cancellation: prefs.echo_cancellation,
            input_volume: prefs.input_volume,
            output_volume: prefs.output_volume,
            input_device: prefs.input_device,
            output_device: prefs.output_device,
        }
    }

    fn init_voice_session(
        &self,
        prefs: &AudioPrefs,
        channel_id: &str,
        community_id: Option<&str>,
        peer_route_blob: Option<&[u8]>,
    ) -> Result<VoiceSessionStartup, VoiceError> {
        session_setup::init_voice_session_impl(
            &self.state,
            prefs,
            channel_id,
            community_id,
            peer_route_blob,
        )
    }

    async fn resolve_peer_route(&self, peer_pubkey_hex: &str) -> Option<Vec<u8>> {
        io_helpers::resolve_peer_route_impl(&self.state, peer_pubkey_hex).await
    }

    async fn resolve_peer_route_from_dht(
        &self,
        community_id: &str,
        peer_pseudonym: &str,
    ) -> Option<Vec<u8>> {
        crate::services::community::routes::resolve_member_route(
            &self.state,
            &self.pool,
            community_id,
            peer_pseudonym,
        )
        .await
    }

    async fn load_member_names(
        &self,
        community_id: Option<&str>,
    ) -> std::collections::HashMap<String, String> {
        io_helpers::load_community_member_names_impl(&self.state, community_id).await
    }

    fn broadcast_media_capabilities(&self, community_id: &str, channel_id: &str) {
        if let Err(e) =
            io_helpers::broadcast_media_capabilities_impl(&self.state, community_id, channel_id)
        {
            tracing::warn!(error = %e, "MediaCapabilities broadcast failed");
        }
    }

    fn emit_local_joined(
        &self,
        channel_id: &str,
        community_id: Option<&str>,
        public_key: &str,
        display_name: &str,
    ) {
        event_mapping::emit_local_joined_impl(
            &self.app_handle,
            channel_id,
            community_id,
            public_key,
            display_name,
        );
        if let Some(community_id) = community_id {
            session_setup::seed_community_media_session(self, community_id, channel_id);
        }
    }

    fn spawn_voice_loops(
        &self,
        public_key: &str,
        transport: Arc<tokio::sync::Mutex<rekindle_voice::transport::VoiceTransport>>,
        muted_flag: Arc<std::sync::atomic::AtomicBool>,
        deafened_flag: Arc<std::sync::atomic::AtomicBool>,
        member_names: std::collections::HashMap<String, String>,
    ) -> Result<(), VoiceError> {
        session_setup::spawn_voice_loops_impl(
            &self.state,
            &self.app_handle,
            &self.pool,
            public_key,
            &transport,
            &muted_flag,
            &deafened_flag,
            member_names,
        )
        .map_err(VoiceError::Session)
    }

    fn restart_audio_devices(&self) -> Result<(), VoiceError> {
        io_helpers::restart_audio_devices_impl(&self.state).map_err(VoiceError::Session)
    }

    fn current_shared_transport(
        &self,
    ) -> Option<Arc<tokio::sync::Mutex<rekindle_voice::transport::VoiceTransport>>> {
        self.state
            .voice_engine
            .lock()
            .as_ref()
            .map(|h| Arc::clone(&h.transport))
    }

    fn current_voice_flags(
        &self,
    ) -> Result<
        (
            Arc<std::sync::atomic::AtomicBool>,
            Arc<std::sync::atomic::AtomicBool>,
        ),
        VoiceError,
    > {
        self.state
            .voice_engine
            .lock()
            .as_ref()
            .map(|h| (Arc::clone(&h.muted_flag), Arc::clone(&h.deafened_flag)))
            .ok_or_else(|| VoiceError::Session("no active voice engine".into()))
    }

    fn active_community_id(&self) -> Option<String> {
        self.state
            .voice_engine
            .lock()
            .as_ref()
            .and_then(|h| h.community_id.clone())
    }

    fn active_channel_info(&self) -> (String, Option<String>) {
        self.state
            .voice_engine
            .lock()
            .as_ref()
            .map(|h| (h.channel_id.clone(), h.community_id.clone()))
            .unwrap_or_default()
    }

    fn send_community_envelope(
        &self,
        community_id: &str,
        envelope: &rekindle_protocol::dht::community::envelope::CommunityEnvelope,
    ) {
        if let Err(e) =
            crate::services::community::send_to_mesh(&self.state, community_id, envelope)
        {
            tracing::debug!(community = %community_id, error = %e,
                "voice adapter: send_community_envelope failed");
        }
    }

    fn log_voice_membership(&self, community_id: &str, channel_id: &str, joined: bool) {
        io_helpers::log_voice_membership_impl(
            &self.state,
            &self.pool,
            community_id,
            channel_id,
            joined,
        );
    }

    fn our_media_route_blob(&self) -> Option<Vec<u8>> {
        // The media-class inbound route (LowLatency + PreferUnordered):
        // the blob peers import to send us voice and video. Never the
        // general route, whose ordered TCP relays head-of-line block
        // realtime media (plan C7.9c).
        state_helpers::our_media_route_blob(&self.state)
    }

    fn my_display_name(&self) -> Option<String> {
        let name = state_helpers::identity_display_name(&self.state);
        (!name.is_empty()).then_some(name)
    }

    fn pre_stage_mcu_channel(
        &self,
    ) -> tokio::sync::mpsc::Receiver<rekindle_voice::transport::VoicePacket> {
        let (tx, rx) = tokio::sync::mpsc::channel(200);
        *self.state.voice_packet_tx.write() = Some(tx);
        rx
    }

    fn begin_mcu_scope(&self) -> Option<Arc<rekindle_lifecycle::SessionScope>> {
        let login = state_helpers::login_scope(&self.state)?;
        let mut ve = self.state.voice_engine.lock();
        let handle = ve.as_mut()?;
        let scope = login.child("voice mcu");
        handle.mcu = Some(Arc::clone(&scope));
        Some(scope)
    }

    fn take_loop_scopes(&self, opts: VoiceShutdownOpts) -> VoiceLoopScopes {
        session_setup::take_loop_scopes_impl(&self.state, opts)
    }

    fn stop_devices_and_clear_engine(&self) {
        let mut ve = self.state.voice_engine.lock();
        if let Some(ref mut handle) = *ve {
            handle.engine.stop_capture();
            handle.engine.stop_playback();
        }
        *ve = None;
    }

    fn stop_audio_devices(&self) {
        let mut ve = self.state.voice_engine.lock();
        if let Some(ref mut handle) = *ve {
            handle.engine.stop_capture();
            handle.engine.stop_playback();
        }
    }

    fn set_voice_engine_devices(&self, input: Option<String>, output: Option<String>) {
        let mut ve = self.state.voice_engine.lock();
        if let Some(ref mut handle) = *ve {
            handle.engine.set_devices(input, output);
        }
    }

    fn voice_engine_device_config(&self) -> (Option<String>, Option<String>) {
        let ve = self.state.voice_engine.lock();
        match ve.as_ref() {
            Some(handle) => {
                let cfg = handle.engine.config();
                (cfg.input_device.clone(), cfg.output_device.clone())
            }
            None => (None, None),
        }
    }

    fn emit_device_changed(&self, device_type: String, device_name: String, reason: String) {
        // No scope: a device can change with no call running at all.
        crate::event_dispatch::emit_subscription(
            &self.app_handle,
            &SubscriptionEvent::Voice(VoiceEvent::DeviceChanged {
                device_type,
                device_name,
                reason,
            }),
        );
    }

    fn emit_system_alert(&self, title: String, body: String) {
        crate::event_dispatch::emit_notification(
            &self.app_handle,
            rekindle_types::subscription_events::NotificationEvent::SystemAlert { title, body },
        );
    }

    async fn stop_active_mcu(&self) {
        let scope = self
            .state
            .voice_engine
            .lock()
            .as_mut()
            .and_then(|handle| handle.mcu.take());
        if let Some(scope) = scope {
            if let Err(stuck) = scope
                .shutdown(rekindle_voice::session::shutdown::LOOP_STOP_DEADLINE)
                .await
            {
                tracing::warn!(%stuck, "voice MCU did not stop in time");
            }
        }
    }

    async fn resolve_member_display_name(
        &self,
        community_id: &str,
        pseudonym: &str,
    ) -> Option<String> {
        let owner_key = state_helpers::current_owner_key(&self.state).ok()?;
        let community = community_id.to_string();
        let pseu = pseudonym.to_string();
        db_call_or_default(&self.pool, move |conn| {
            rekindle_db::repo::members::display_name(conn, &owner_key, &community, &pseu)
        })
        .await
    }
}
