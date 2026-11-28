//! `impl VoiceSessionDeps for HostVoiceDeps`: the voice session's surface
//! over Host state (the desktop's `voice_adapter::deps_impl`, ported,
//! plan E4.1 S5a2; lead ruling 2026-11-26: one engine, the Host's).

use std::collections::BTreeMap;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use async_trait::async_trait;
use rekindle_codec::community::envelope::CommunityEnvelope;
use rekindle_lifecycle::SessionScope;
use rekindle_secrets::media_sender_key::keyring::ChannelSenderKeys;
use rekindle_secrets::media_sender_key::session;
use rekindle_types::subscription_events::{
    NotificationEvent, SubscriptionEvent, VoiceEvent, VoiceScope,
};
use rekindle_voice::transport::{VoicePacket, VoiceTransport};
use rekindle_voice::{
    AudioPrefs, CallMediaKeys, MediaKeySource, VoiceError, VoiceIdentity, VoiceLoopScopes,
    VoicePeer, VoiceSessionDeps, VoiceSessionEvent, VoiceSessionStartup, VoiceShutdownOpts,
};

use super::{events, setup, HostVoice};
use crate::daemon::dispatch::DaemonContext;
use crate::host::adapters::governance::HostGovernance;

type SharedTransport = Arc<tokio::sync::Mutex<VoiceTransport>>;

/// The voice session's deps over one Host's context; `prefs` are the
/// unlocked identity's audio preferences read at the join.
#[derive(Clone)]
pub struct HostVoiceDeps {
    ctx: Arc<DaemonContext>,
    prefs: Option<AudioPrefs>,
    initial_controls: (bool, bool),
}

impl HostVoiceDeps {
    #[must_use]
    pub(super) fn new(ctx: Arc<DaemonContext>, prefs: Option<AudioPrefs>) -> Self {
        Self {
            ctx,
            prefs,
            initial_controls: (false, false),
        }
    }

    pub(super) fn with_initial_controls(mut self, muted: bool, deafened: bool) -> Self {
        self.initial_controls = (muted, deafened);
        self
    }

    pub(super) fn initial_controls(&self) -> (bool, bool) {
        self.initial_controls
    }

    pub(super) fn ctx(&self) -> &Arc<DaemonContext> {
        &self.ctx
    }

    pub(super) fn voice(&self) -> &HostVoice {
        &self.ctx.voice
    }

    /// Our voice signing key: the community pseudonym, or the account key
    /// for a 1:1 call.
    pub(super) fn signing_key(
        &self,
        community_id: Option<&str>,
    ) -> Option<rekindle_secrets::ed25519_dalek::SigningKey> {
        let secret = self.ctx.identity_secret()?;
        Some(match community_id {
            Some(c) => rekindle_secrets::derive::derive_community_pseudonym(&secret, c),
            None => rekindle_secrets::ed25519_dalek::SigningKey::from_bytes(&secret),
        })
    }

    pub(super) fn sign(&self, community_id: &str, message: &[u8]) -> Option<Vec<u8>> {
        let key = self.signing_key(Some(community_id))?;
        Some(rekindle_secrets::derive::sign_with_pseudonym(&key, message).to_vec())
    }

    pub(super) fn governance(&self) -> HostGovernance {
        HostGovernance::new(Arc::clone(&self.ctx))
    }

    pub(super) fn channel<R>(
        &self,
        community_id: &str,
        channel_id: &str,
        read: impl FnOnce(&crate::host::community::state::ChannelInfo) -> R,
    ) -> Option<R> {
        let map = self.ctx.communities.map().read();
        let community = map.get(community_id)?;
        community
            .channels
            .iter()
            .find(|c| c.id == channel_id)
            .map(read)
    }

    fn owner(&self) -> Option<String> {
        (self.ctx.session.read().as_ref()).map(|s| s.identity.public_key_hex.clone())
    }

    fn with_engine(&self, f: impl FnOnce(&mut rekindle_voice::VoiceEngine)) {
        if let Some(engine) = self.voice().live.lock().as_mut() {
            f(&mut engine.engine);
        }
    }

    pub(super) fn publish(&self, event: VoiceEvent) {
        self.ctx.publish_event(SubscriptionEvent::Voice(event));
    }
}

impl MediaKeySource for HostVoiceDeps {
    fn channel_sender_keys(&self, community_id: &str, channel_id: &str) -> Arc<ChannelSenderKeys> {
        self.voice().sender_keys.keys(community_id, channel_id)
    }

    fn call_media(&self, peer_pubkey: &str) -> Option<CallMediaKeys> {
        let call = self.voice().call.lock();
        let call = call.as_ref().filter(|call| call.peer == peer_pubkey)?;
        Some(CallMediaKeys {
            secret: call.keys.secret.clone(),
            sender: Arc::clone(&call.keys.sender),
        })
    }

    fn clock(&self) -> Arc<dyn rekindle_utils::Clock> {
        self.voice().clock()
    }
}

#[async_trait]
impl VoiceSessionDeps for HostVoiceDeps {
    fn sign_media_session_key(
        &self,
        community_id: &str,
        channel_id: &str,
        key: &[u8],
    ) -> Option<Vec<u8>> {
        self.sign(
            community_id,
            &session::session_key_message(community_id, channel_id, key),
        )
    }

    fn owner_key(&self) -> Result<String, VoiceError> {
        self.owner().ok_or(VoiceError::IdentityNotLoaded)
    }

    fn voice_self_identity(&self, community_id: Option<&str>) -> String {
        match community_id {
            Some(c) => self.ctx.my_pseudonym(c),
            None => self.owner().unwrap_or_default(),
        }
    }

    fn identity_secret(&self) -> Result<[u8; 32], VoiceError> {
        self.ctx
            .identity_secret()
            .ok_or(VoiceError::IdentityNotLoaded)
    }

    fn voice_engine_present(&self) -> bool {
        self.voice().engine_present()
    }

    fn set_voice_engine_muted(&self, muted: bool) {
        self.voice().set_muted(muted);
    }

    fn set_voice_engine_deafened(&self, deafened: bool) {
        self.voice().set_deafened(deafened);
    }

    fn pre_stage_voice_channel(&self) {
        let (tx, rx) = tokio::sync::mpsc::channel(200);
        *self.voice().packet_tx.write() = Some(tx);
        *self.voice().packet_rx_staged.lock() = Some(rx);
    }

    fn clear_voice_channels(&self) {
        *self.voice().packet_tx.write() = None;
        *self.voice().packet_rx_staged.lock() = None;
        *self.voice().report_tx.write() = None;
    }

    /// The members the Host reaches: roster ∩ `M`/6 ∩ beat present (plan
    /// E3.4 S4).
    fn voice_peers(&self, community_id: &str, _channel_id: &str) -> Vec<VoicePeer> {
        (self
            .governance()
            .reachable_members(community_id)
            .into_iter())
        .map(|(pseudonym, route_blob)| VoicePeer {
            pseudonym,
            display_name: String::new(),
            route_blob: Some(route_blob),
        })
        .collect()
    }

    fn channel_is_stage(&self, community_id: &str, channel_id: &str) -> bool {
        self.channel(community_id, channel_id, |c| {
            c.channel_type == crate::host::community::state::ChannelType::Stage
        })
        .unwrap_or(false)
    }

    fn request_media_key(&self, community_id: &str, channel_id: &str, sender: &str, index: u64) {
        super::io::request_media_key(self, community_id, channel_id, sender, index);
    }

    fn send_receiver_report(&self, peer_pubkey_hex: &str, wire: Vec<u8>) {
        let Some(transport) = self.current_shared_transport() else {
            return;
        };
        let peer = peer_pubkey_hex.to_string();
        let clock = self.clock();
        self.ctx
            .unlock_scope_or_closed()
            .spawn_or_drop("voice receiver report", async move {
                let is_feedback = wire.first() == Some(&rekindle_voice::media_frame::FEEDBACK_TAG);
                let queued = clock.monotonic();
                let guard = transport.lock().await;
                let lock_wait = clock.monotonic().saturating_duration_since(queued);
                let stats = Arc::clone(guard.media().feedback_stats());
                let sending = clock.monotonic();
                let result = guard.send_bytes_to_peer(&peer, wire).await;
                if is_feedback {
                    stats.note_handed(
                        &peer,
                        lock_wait,
                        clock.monotonic().saturating_duration_since(sending),
                        result.is_ok(),
                    );
                }
                if let Err(e) = result {
                    tracing::debug!(peer = %peer, error = %e, "receiver report not delivered");
                }
            });
    }

    fn we_are_stage_speaker(&self, community_id: &str, channel_id: &str, ours: &str) -> bool {
        self.channel(community_id, channel_id, |c| {
            c.stage_moderator.as_deref() == Some(ours) || c.stage_speakers.iter().any(|s| s == ours)
        })
        .unwrap_or(false)
    }

    fn sender_is_stage_speaker(&self, community_id: &str, channel_id: &str, sender: &str) -> bool {
        self.channel(community_id, channel_id, |c| {
            c.stage_speakers.iter().any(|s| s == sender)
        })
        .unwrap_or(true)
    }

    fn record_packet_drop(&self) {
        self.voice().note_drop();
    }

    fn emit_voice_event(&self, event: VoiceSessionEvent) {
        let Some(scope) = self.voice().session() else {
            return;
        };
        events::emit(self, scope, event);
    }

    fn scope(&self) -> Arc<SessionScope> {
        self.ctx.unlock_scope_or_closed()
    }

    fn current_identity(&self) -> Result<VoiceIdentity, VoiceError> {
        let session = self.ctx.session.read();
        let identity = &session
            .as_ref()
            .ok_or(VoiceError::IdentityNotLoaded)?
            .identity;
        Ok(VoiceIdentity {
            public_key: identity.public_key_hex.clone(),
            display_name: identity.display_name.clone(),
        })
    }

    fn check_not_in_call(&self, _: &str) -> Result<(), VoiceError> {
        if self.voice().engine_present() {
            return Err(VoiceError::Session("voice session already running".into()));
        }
        Ok(())
    }

    fn audio_prefs(&self) -> Result<AudioPrefs, VoiceError> {
        self.prefs.clone().ok_or(VoiceError::IdentityNotLoaded)
    }

    fn init_voice_session(
        &self,
        prefs: &AudioPrefs,
        channel_id: &str,
        community_id: Option<&str>,
        peer_route_blob: Option<&[u8]>,
    ) -> Result<VoiceSessionStartup, VoiceError> {
        setup::init_voice_session(self, prefs, channel_id, community_id, peer_route_blob)
    }

    async fn resolve_peer_route(&self, peer_pubkey_hex: &str) -> Option<Vec<u8>> {
        use rekindle_transport::operations::dm::route;
        let node = self.ctx.transport.read().clone()?;
        if let Some(cached) = route::cached(&node, peer_pubkey_hex) {
            return Some(cached);
        }
        let host = crate::host::adapters::friends::HostFriends::of(&self.ctx);
        route::fetch_from_profile(&host, peer_pubkey_hex).await
    }

    fn broadcast_media_capabilities(&self, community_id: &str, channel_id: &str) {
        super::io::broadcast_media_capabilities(self, community_id, channel_id);
    }

    fn emit_local_joined(
        &self,
        session: &rekindle_voice::session_state::VoiceSessionState,
        channel_id: &str,
        community_id: Option<&str>,
        key: &str,
        name: &str,
    ) {
        events::emit_local_joined(self, session, channel_id, community_id, key, name);
    }

    fn spawn_voice_loops(
        &self,
        public_key: &str,
        transport: SharedTransport,
        muted_flag: Arc<AtomicBool>,
        deafened_flag: Arc<AtomicBool>,
    ) -> Result<(), VoiceError> {
        setup::spawn_voice_loops(self, public_key, &transport, &muted_flag, &deafened_flag)
    }

    fn restart_audio_devices(&self) -> Result<(), VoiceError> {
        let mut live = self.voice().live.lock();
        live.as_mut()
            .ok_or(VoiceError::NotConnected)?
            .engine
            .start_audio()
    }

    fn current_shared_transport(&self) -> Option<SharedTransport> {
        self.voice().transport()
    }

    fn current_voice_flags(&self) -> Result<(Arc<AtomicBool>, Arc<AtomicBool>), VoiceError> {
        (self.voice().live.lock().as_ref())
            .map(|s| (Arc::clone(&s.muted), Arc::clone(&s.deafened)))
            .ok_or_else(|| VoiceError::Session("no active voice engine".into()))
    }

    fn active_community_id(&self) -> Option<String> {
        self.active_channel_info().1
    }

    fn active_channel_info(&self) -> (String, Option<String>) {
        match self.voice().session() {
            Some(VoiceScope::Community { community, channel }) => (channel, Some(community)),
            Some(VoiceScope::Dm { peer_key }) => (peer_key, None),
            None => (String::new(), None),
        }
    }

    fn send_community_envelope(&self, community_id: &str, envelope: &CommunityEnvelope) {
        self.governance().send_to_mesh_impl(community_id, envelope);
    }

    async fn send_channel_envelope_now(
        &self,
        community_id: &str,
        channel_id: &str,
        envelope: &CommunityEnvelope,
    ) {
        super::io::send_to_channel_peers_now(self, community_id, channel_id, envelope).await;
    }

    fn log_voice_membership(&self, community_id: &str, channel_id: &str, joined: bool) {
        super::io::log_voice_membership(self, community_id, channel_id, joined);
    }

    /// The media-class route (LowLatency, unordered), never the general
    /// route (plan C7.9c).
    fn our_media_route_blob(&self) -> Option<Vec<u8>> {
        self.ctx.transport.read().as_ref()?.media_route_blob()
    }

    fn my_display_name(&self) -> Option<String> {
        let name = self.ctx.identity_display_name();
        (!name.is_empty()).then_some(name)
    }

    fn pre_stage_mcu_channel(&self) -> tokio::sync::mpsc::Receiver<VoicePacket> {
        let (tx, rx) = tokio::sync::mpsc::channel(200);
        *self.voice().packet_tx.write() = Some(tx);
        rx
    }

    fn begin_mcu_scope(&self) -> Option<Arc<SessionScope>> {
        let unlock = self.ctx.unlock_scope_or_closed();
        let mut live = self.voice().live.lock();
        let session = live.as_mut()?;
        let scope = unlock.child("voice mcu");
        session.mcu = Some(Arc::clone(&scope));
        Some(scope)
    }

    async fn stop_active_mcu(&self) {
        let scope = self.voice().live.lock().as_mut().and_then(|s| s.mcu.take());
        if let Some(scope) = scope {
            if let Err(stuck) = scope
                .shutdown(rekindle_voice::session::shutdown::LOOP_STOP_DEADLINE)
                .await
            {
                tracing::warn!(%stuck, "voice MCU did not stop in time");
            }
        }
    }

    fn take_loop_scopes(&self, opts: VoiceShutdownOpts) -> VoiceLoopScopes {
        setup::take_loop_scopes(self, opts)
    }

    fn stop_devices_and_clear_engine(&self) {
        let scope = self.voice().session();
        if let Some(VoiceScope::Community { community, channel }) = &scope {
            if let Some(state) = self.voice().channel_state(community, channel) {
                state.end(|status| {
                    self.publish(VoiceEvent::MediaReady {
                        scope: VoiceScope::Community {
                            community: community.clone(),
                            channel: channel.clone(),
                        },
                        ready: status.ready,
                        reason: status.reason,
                    });
                });
            }
        }
        let ended = self.voice().end();
        for key in [
            events::SESSION_STATE,
            events::SELF_STATE,
            super::signaling_events::ROSTER_STATE,
        ] {
            self.ctx.journal.clear_state(key);
        }
        if let (Some(scope), Some(pseudonym)) = (ended, self.owner()) {
            self.publish(VoiceEvent::Left { scope, pseudonym });
        }
    }

    async fn end_channel_session(&self, community_id: &str, channel_id: &str) {
        // The session's keys end with it, and the keys still queued for it
        // (plans C7.20, C7.29, E2.5); a rejoin draws new ones.
        let outbox = super::carrier::HostMediaKeyOutbox(Arc::downgrade(&self.ctx));
        rekindle_voice::signaling::end_media_key_session(
            &self.voice().sender_keys,
            &outbox,
            community_id,
            channel_id,
        )
        .await;
    }

    fn stop_audio_devices(&self) {
        if let Some(s) = self.voice().live.lock().as_mut() {
            s.engine.stop_capture();
            s.engine.stop_playback();
        }
    }

    fn set_voice_engine_devices(&self, input: Option<String>, output: Option<String>) {
        self.with_engine(|e| e.set_devices(input, output));
    }

    fn voice_engine_device_config(&self) -> (Option<String>, Option<String>) {
        let live = self.voice().live.lock();
        live.as_ref().map(|s| &s.engine).map_or((None, None), |e| {
            (
                e.config().input_device.clone(),
                e.config().output_device.clone(),
            )
        })
    }

    fn set_voice_engine_input_channels(&self, input_channels: BTreeMap<String, Vec<u16>>) {
        self.with_engine(|e| e.set_input_channels(input_channels));
    }

    fn emit_device_changed(&self, device_type: String, device_name: String, notice: String) {
        // No scope: a device can change with no call running.
        self.publish(VoiceEvent::DeviceChanged {
            device_type,
            device_name,
            notice,
        });
    }

    fn emit_system_alert(&self, title: String, body: String) {
        self.ctx.publish_event(SubscriptionEvent::Notification(
            NotificationEvent::SystemAlert {
                title,
                body,
                subject: None,
            },
        ));
    }

    async fn resolve_member_display_name(
        &self,
        community_id: &str,
        pseudonym: &str,
    ) -> Option<String> {
        let owner = self.owner()?;
        let db = self.ctx.db.current().ok()?;
        let (cid, pseu) = (community_id.to_string(), pseudonym.to_string());
        db.call(move |conn| rekindle_db::repo::members::display_name(conn, &owner, &cid, &pseu))
            .await
            .unwrap_or_else(|e| {
                tracing::warn!(error = %e, "member display name unreadable");
                None
            })
    }
}
