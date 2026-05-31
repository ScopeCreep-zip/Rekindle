//! Phase 14.r split — `impl VoiceSessionDeps for VoiceAdapter`.
//!
//! All voice-session trait surface in one place. Methods either
//! read/mutate AppState directly (under parking_lot or atomics) or
//! delegate into the helper modules (`session_setup`, `event_mapping`,
//! `io_helpers`) for bodies that would otherwise blow the file-size
//! cap.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use async_trait::async_trait;
use rekindle_voice::{
    AudioPrefs, CallKeyInfo, VoiceError, VoiceIdentity, VoicePeer, VoiceSessionDeps,
    VoiceSessionEvent, VoiceSessionStartup, VoiceShutdownHandles, VoiceShutdownOpts,
};

use super::{event_mapping, io_helpers, session_setup, VoiceAdapter};
use crate::channels::VoiceEvent;
use crate::db_helpers::db_call_or_default;
use crate::state_helpers;

#[async_trait]
impl VoiceSessionDeps for VoiceAdapter {
    fn owner_key(&self) -> Result<String, VoiceError> {
        let key = state_helpers::owner_key_or_default(&self.state);
        if key.is_empty() {
            Err(VoiceError::IdentityNotLoaded)
        } else {
            Ok(key)
        }
    }

    fn identity_secret(&self) -> Result<[u8; 32], VoiceError> {
        let guard = self.state.identity_secret.lock();
        guard.as_ref().copied().ok_or(VoiceError::IdentityNotLoaded)
    }

    fn voice_engine_present(&self) -> bool {
        self.state.voice_engine.lock().is_some()
    }

    fn set_voice_engine_muted(&self, muted: bool) {
        let mut ve = self.state.voice_engine.lock();
        if let Some(ref mut handle) = *ve {
            handle.engine.set_muted(muted);
            handle.muted_flag.store(muted, Ordering::Relaxed);
        }
    }

    fn set_voice_engine_deafened(&self, deafened: bool) {
        let mut ve = self.state.voice_engine.lock();
        if let Some(ref mut handle) = *ve {
            handle.engine.set_deafened(deafened);
            handle.deafened_flag.store(deafened, Ordering::Relaxed);
        }
    }

    fn pre_stage_voice_channel(&self) {
        let (tx, rx) = tokio::sync::mpsc::channel(200);
        *self.state.voice_packet_tx.write() = Some(tx);
        *self.state.voice_packet_rx_staged.lock() = Some(rx);
    }

    fn clear_voice_channels(&self) {
        *self.state.voice_packet_tx.write() = None;
        *self.state.voice_packet_rx_staged.lock() = None;
    }

    fn community_voice_mek(&self, community_id: &str) -> Option<[u8; 32]> {
        let cache = self.state.mek_cache.lock();
        cache.get(community_id).map(|mek| *mek.as_bytes())
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
        let communities = self.state.communities.read();
        let Some(community) = communities.get(community_id) else {
            return false;
        };
        let Some(channel) = community.channels.iter().find(|ch| ch.id == channel_id) else {
            return false;
        };
        matches!(channel.channel_type, crate::state::ChannelType::Stage)
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
        channel
            .stage_speakers
            .iter()
            .any(|s| s == sender_pseudonym)
    }

    fn call_key_for_peer(&self, peer_pubkey: &str) -> Option<CallKeyInfo> {
        self.state
            .active_calls
            .list_all()
            .into_iter()
            .find(|c| c.peer_pubkey == peer_pubkey)
            .and_then(|c| {
                c.call_key.map(|k| CallKeyInfo {
                    call_key: k,
                    peer_pubkey: peer_pubkey.to_string(),
                })
            })
    }

    fn record_packet_drop(&self) {
        self.state.voice_pkt_drops.fetch_add(1, Ordering::Relaxed);
    }

    fn packet_drops(&self) -> u64 {
        self.state.voice_pkt_drops.load(Ordering::Relaxed)
    }

    fn emit_voice_event(&self, event: VoiceSessionEvent) {
        crate::event_dispatch::dispatch(
            &self.app_handle,
            "voice-event",
            event_mapping::map(event),
        );
    }

    fn register_background_handle(&self, handle: tokio::task::JoinHandle<()>) {
        let wrapped = tauri::async_runtime::spawn(async move {
            let _ = handle.await;
        });
        self.state.background_handles.lock().push(wrapped);
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

    fn our_route_blob(&self) -> Vec<u8> {
        state_helpers::our_route_blob(&self.state).unwrap_or_default()
    }

    fn pre_stage_mcu_channel(
        &self,
    ) -> tokio::sync::mpsc::Receiver<rekindle_voice::transport::VoicePacket> {
        let (tx, rx) = tokio::sync::mpsc::channel(200);
        *self.state.voice_packet_tx.write() = Some(tx);
        rx
    }

    fn register_mcu_task(
        &self,
        shutdown_tx: tokio::sync::mpsc::Sender<()>,
        handle: tokio::task::JoinHandle<()>,
    ) {
        let mut ve = self.state.voice_engine.lock();
        if let Some(ref mut h) = *ve {
            h.mcu_loop_shutdown = Some(shutdown_tx);
            h.mcu_loop_handle = Some(handle);
        }
    }

    fn take_shutdown_handles(&self, opts: VoiceShutdownOpts) -> VoiceShutdownHandles {
        session_setup::take_shutdown_handles_impl(&self.state, opts)
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
        crate::event_dispatch::dispatch(&self.app_handle, 
            "voice-event",
            VoiceEvent::DeviceChanged {
                device_type,
                device_name,
                reason,
            },
        );
    }

    fn emit_system_alert(&self, title: String, body: String) {
        crate::event_dispatch::dispatch(&self.app_handle, 
            "notification-event",
            crate::channels::NotificationEvent::SystemAlert { title, body },
        );
    }

    async fn stop_active_mcu(&self) {
        let (mcu_tx, mcu_h) = {
            let mut ve = self.state.voice_engine.lock();
            if let Some(ref mut handle) = *ve {
                (
                    handle.mcu_loop_shutdown.take(),
                    handle.mcu_loop_handle.take(),
                )
            } else {
                (None, None)
            }
        };
        if let Some(tx) = mcu_tx {
            let _ = tx.send(()).await;
        }
        if let Some(h) = mcu_h {
            let _ = h.await;
        }
    }

    async fn resolve_member_display_name(
        &self,
        community_id: &str,
        pseudonym: &str,
    ) -> Option<String> {
        let community = community_id.to_string();
        let pseu = pseudonym.to_string();
        let names: HashMap<String, String> = db_call_or_default(&self.pool, move |conn| {
            let mut stmt = conn.prepare(
                "SELECT display_name FROM community_members
                 WHERE community_id = ?1 AND pseudonym_key = ?2 LIMIT 1",
            )?;
            let mut rows = stmt.query(rusqlite::params![community, pseu])?;
            let mut map = HashMap::new();
            if let Some(row) = rows.next()? {
                let name: Option<String> = row.get(0)?;
                if let Some(n) = name {
                    map.insert(String::new(), n);
                }
            }
            Ok(map)
        })
        .await;
        names.into_values().next()
    }
}
