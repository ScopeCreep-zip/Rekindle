//! A Leave followed at once by a rejoin is a new channel session (plans
//! C7.20, C7.29; Linux walk row M114).
//!
//! The Leave button calls [`leave_voice`], which tore the session down
//! without ending the channel's keyring: the rejoin announced the same
//! session key, so its `VoiceJoin` was byte-identical to the first join's
//! and every peer's gossip dedup (a content hash over a `Control`
//! envelope, `rekindle_gossip::broadcast::extract_mesh_dedup_key`)
//! dropped it, and every presence-reconcile re-send after it.

use std::collections::BTreeMap;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::Mutex;
use rekindle_codec::capnp_envelope::encode_community_envelope;
use rekindle_codec::community::envelope::{CommunityEnvelope, ControlPayload};
use rekindle_codec::dedup::DedupCache;
use rekindle_secrets::media_sender_key::keyring::{ChannelSenderKeyStore, ChannelSenderKeys};

use crate::session_deps::{
    AudioPrefs, CallMediaKeys, MediaKeySource, VoiceIdentity, VoiceLoopScopes, VoicePeer,
    VoiceSessionDeps, VoiceSessionEvent, VoiceSessionStartup, VoiceShutdownOpts,
};
use crate::signaling::MediaKeyOutbox;
use crate::transport::{VoicePacket, VoiceTransport};
use crate::{VoiceError, VoiceFrameSender};
use rekindle_lifecycle::SessionScope;

const COMMUNITY: &str = "community";
const CHANNEL: &str = "voice";

struct NoQueuedKeys;

#[async_trait]
impl MediaKeyOutbox for NoQueuedKeys {
    async fn queue(&self, _: &str, _: &str, _: &str, _: Vec<u8>) -> Result<(), String> {
        Ok(())
    }

    async fn discard_session(&self, _: &str) -> Result<usize, String> {
        Ok(0)
    }
}

/// The joiner: one community voice channel, one media route, and the
/// envelopes it sends to the community mesh.
#[derive(Default)]
struct Joiner {
    keys: ChannelSenderKeyStore,
    mesh: Mutex<Vec<CommunityEnvelope>>,
    /// The leave path's steps, in order.
    steps: Mutex<Vec<&'static str>>,
    /// The current session's egress scope, as the engine handle holds it.
    egress: Mutex<Option<Arc<SessionScope>>>,
}

/// A frame sender that accepts every datagram.
struct Sink;

#[async_trait]
impl VoiceFrameSender for Sink {
    async fn send_voice_frame(&self, _: &[u8], _: Vec<u8>) -> Result<(), VoiceError> {
        Ok(())
    }
}

impl MediaKeySource for Joiner {
    fn channel_sender_keys(&self, community_id: &str, channel_id: &str) -> Arc<ChannelSenderKeys> {
        self.keys.keys(community_id, channel_id)
    }

    fn call_media(&self, _: &str) -> Option<CallMediaKeys> {
        None
    }

    fn clock(&self) -> Arc<dyn rekindle_utils::Clock> {
        Arc::new(rekindle_utils::SystemClock)
    }
}

#[async_trait]
impl VoiceSessionDeps for Joiner {
    fn sign_media_session_key(&self, _: &str, _: &str, _: &[u8]) -> Option<Vec<u8>> {
        Some(vec![7; 64])
    }
    fn owner_key(&self) -> Result<String, VoiceError> {
        Ok("owner".into())
    }
    fn voice_self_identity(&self, _: Option<&str>) -> String {
        unreachable!("not on the leave path")
    }
    fn identity_secret(&self) -> Result<[u8; 32], VoiceError> {
        unreachable!("not on the leave path")
    }
    fn voice_engine_present(&self) -> bool {
        unreachable!("not on the leave path")
    }
    fn set_voice_engine_muted(&self, _: bool) {
        unreachable!("not on the leave path")
    }
    fn set_voice_engine_deafened(&self, _: bool) {
        unreachable!("not on the leave path")
    }
    fn pre_stage_voice_channel(&self) {
        unreachable!("not on the leave path")
    }
    fn clear_voice_channels(&self) {}
    fn voice_peers(&self, _: &str, _: &str) -> Vec<VoicePeer> {
        unreachable!("not on the leave path")
    }
    fn channel_is_stage(&self, _: &str, _: &str) -> bool {
        unreachable!("not on the leave path")
    }
    fn request_media_key(&self, _: &str, _: &str, _: &str, _: u64) {
        unreachable!("not on the leave path")
    }
    fn send_receiver_report(&self, _: &str, _: Vec<u8>) {
        unreachable!("not on the leave path")
    }
    fn we_are_stage_speaker(&self, _: &str, _: &str, _: &str) -> bool {
        unreachable!("not on the leave path")
    }
    fn sender_is_stage_speaker(&self, _: &str, _: &str, _: &str) -> bool {
        unreachable!("not on the leave path")
    }
    fn record_packet_drop(&self) {
        unreachable!("not on the leave path")
    }
    fn emit_voice_event(&self, _: VoiceSessionEvent) {}
    fn scope(&self) -> Arc<rekindle_lifecycle::SessionScope> {
        unreachable!("not on the leave path")
    }
    fn current_identity(&self) -> Result<VoiceIdentity, VoiceError> {
        unreachable!("not on the leave path")
    }
    fn check_not_in_call(&self, _: &str) -> Result<(), VoiceError> {
        unreachable!("not on the leave path")
    }
    fn audio_prefs(&self) -> Result<AudioPrefs, VoiceError> {
        unreachable!("not on the leave path")
    }
    fn init_voice_session(
        &self,
        _: &AudioPrefs,
        _: &str,
        _: Option<&str>,
        _: Option<&[u8]>,
    ) -> Result<VoiceSessionStartup, VoiceError> {
        unreachable!("not on the leave path")
    }
    async fn resolve_peer_route(&self, _: &str) -> Option<Vec<u8>> {
        unreachable!("not on the leave path")
    }
    fn broadcast_media_capabilities(&self, _: &str, _: &str) {
        unreachable!("not on the leave path")
    }
    fn emit_local_joined(
        &self,
        _: &crate::session_state::VoiceSessionState,
        _: &str,
        _: Option<&str>,
        _: &str,
        _: &str,
    ) {
        unreachable!("not on the leave path")
    }
    fn spawn_voice_loops(
        &self,
        _: &str,
        _: Arc<tokio::sync::Mutex<VoiceTransport>>,
        _: Arc<AtomicBool>,
        _: Arc<AtomicBool>,
    ) -> Result<(), VoiceError> {
        unreachable!("not on the leave path")
    }
    fn restart_audio_devices(&self) -> Result<(), VoiceError> {
        unreachable!("not on the leave path")
    }
    fn current_shared_transport(&self) -> Option<Arc<tokio::sync::Mutex<VoiceTransport>>> {
        unreachable!("not on the leave path")
    }
    fn current_voice_flags(&self) -> Result<(Arc<AtomicBool>, Arc<AtomicBool>), VoiceError> {
        unreachable!("not on the leave path")
    }
    fn active_community_id(&self) -> Option<String> {
        unreachable!("not on the leave path")
    }
    fn active_channel_info(&self) -> (String, Option<String>) {
        (CHANNEL.into(), Some(COMMUNITY.into()))
    }
    fn send_community_envelope(&self, _: &str, envelope: &CommunityEnvelope) {
        self.mesh.lock().push(envelope.clone());
    }
    async fn send_channel_envelope_now(&self, _: &str, _: &str, envelope: &CommunityEnvelope) {
        if matches!(
            envelope.as_control(),
            Some(ControlPayload::VoiceLeave { .. })
        ) {
            self.steps.lock().push("leave sent to the roster");
        }
    }
    fn log_voice_membership(&self, _: &str, _: &str, _: bool) {}
    fn our_media_route_blob(&self) -> Option<Vec<u8>> {
        // The same media route across the leave, as on the walk.
        Some(vec![1, 2, 3])
    }
    fn my_display_name(&self) -> Option<String> {
        Some("walk".into())
    }
    fn pre_stage_mcu_channel(&self) -> tokio::sync::mpsc::Receiver<VoicePacket> {
        unreachable!("not on the leave path")
    }
    fn begin_mcu_scope(&self) -> Option<Arc<rekindle_lifecycle::SessionScope>> {
        unreachable!("not on the leave path")
    }
    async fn stop_active_mcu(&self) {
        unreachable!("not on the leave path")
    }
    fn take_loop_scopes(&self, opts: VoiceShutdownOpts) -> VoiceLoopScopes {
        VoiceLoopScopes {
            loops: None,
            monitor: None,
            mcu: None,
            egress: if opts.stop_devices {
                self.egress.lock().take()
            } else {
                None
            },
        }
    }
    fn stop_devices_and_clear_engine(&self) {}
    async fn end_channel_session(&self, community_id: &str, channel_id: &str) {
        self.steps.lock().push("session ended");
        crate::signaling::end_media_key_session(
            &self.keys,
            &NoQueuedKeys,
            community_id,
            channel_id,
        )
        .await;
    }
    fn stop_audio_devices(&self) {
        unreachable!("not on the leave path")
    }
    fn set_voice_engine_devices(&self, _: Option<String>, _: Option<String>) {
        unreachable!("not on the leave path")
    }
    fn voice_engine_device_config(&self) -> (Option<String>, Option<String>) {
        unreachable!("not on the leave path")
    }
    fn set_voice_engine_input_channels(&self, _: BTreeMap<String, Vec<u16>>) {
        unreachable!("not on the leave path")
    }
    fn emit_device_changed(&self, _: String, _: String, _: String) {
        unreachable!("not on the leave path")
    }
    fn emit_system_alert(&self, _: String, _: String) {
        unreachable!("not on the leave path")
    }
    async fn resolve_member_display_name(&self, _: &str, _: &str) -> Option<String> {
        unreachable!("not on the leave path")
    }
}

/// The joiner's latest `VoiceJoin` to the mesh, as it goes on the wire.
fn last_join(joiner: &Joiner) -> (Vec<u8>, Vec<u8>) {
    let mesh = joiner.mesh.lock();
    let join = mesh
        .iter()
        .rev()
        .find(|e| matches!(e.as_control(), Some(ControlPayload::VoiceJoin { .. })))
        .cloned()
        .expect("a VoiceJoin was sent");
    let Some(ControlPayload::VoiceJoin { session_key, .. }) = join.as_control() else {
        unreachable!("matched above");
    };
    let bytes = encode_community_envelope(&join).expect("encodes");
    (bytes, session_key.key.clone())
}

/// The peer's gossip dedup admits the envelope (the key is a content hash
/// over these bytes, so equal bytes are one key).
fn peer_admits(peer: &mut DedupCache, bytes: &[u8]) -> bool {
    !peer.check_and_insert(COMMUNITY, "walk", &hex::encode(bytes))
}

#[tokio::test]
async fn leave_then_rejoin_announces_a_join_the_peer_admits() {
    let joiner = Arc::new(Joiner::default());
    let mut peer = DedupCache::new(1024);

    // First session: the join the peer applies.
    crate::session::reannounce_voice_route(&joiner);
    let (first, first_key) = last_join(&joiner);
    assert!(peer_admits(&mut peer, &first));

    // Leave, and rejoin 0.45 s later on the same media route.
    crate::session::leave_voice(&joiner).await.expect("leaves");
    crate::session::reannounce_voice_route(&joiner);
    let (rejoin, rejoin_key) = last_join(&joiner);

    assert_ne!(rejoin_key, first_key, "a rejoin is a new channel session");
    assert!(
        peer_admits(&mut peer, &rejoin),
        "the peer must not drop the rejoin as the first join's duplicate"
    );
}

/// Linux walk row M25: a leave 0.2 s after a rejoin. The rejoin's key
/// deliveries were still queued, and one was handed to Veilid after the
/// leave went out (delivered 0.5 s after it). The session's keys and its
/// queued deliveries must end before the leave reaches the roster, so no
/// key of the session is attempted after its leave.
#[tokio::test]
async fn leave_right_after_a_rejoin_ends_the_session_before_the_leave() {
    let joiner = Arc::new(Joiner::default());
    crate::session::reannounce_voice_route(&joiner);
    crate::session::leave_voice(&joiner).await.expect("leaves");
    crate::session::reannounce_voice_route(&joiner);
    joiner.steps.lock().clear();

    crate::session::leave_voice(&joiner).await.expect("leaves");

    let steps = joiner.steps.lock().clone();
    assert_eq!(
        steps.first().copied(),
        Some("session ended"),
        "the session's key deliveries must end before its leave goes out: {steps:?}"
    );
    assert!(steps.contains(&"leave sent to the roster"), "{steps:?}");
}

/// Plan CL1 (Linux soak: every call left its media behind): a session's
/// egress drivers end with it. Nothing else stops the driver of a peer
/// still on the roster, so each ended call kept one pacing (and padding)
/// to the old route until logout.
#[tokio::test]
async fn ended_sessions_leave_no_egress_driver_running() {
    let login = SessionScope::new("login", Arc::new(|_| {}));
    let joiner = Arc::new(Joiner::default());
    for _ in 0..3 {
        let egress = login.child("voice media egress");
        let mut transport =
            VoiceTransport::new(CHANNEL.into(), Arc::new(rekindle_utils::SystemClock));
        transport.set_signing_key(ed25519_dalek::SigningKey::from_bytes(&[3; 32]));
        transport.connect(
            Arc::new(Sink),
            b"route",
            vec![1; 32],
            "peer",
            Arc::clone(&egress),
        );
        *joiner.egress.lock() = Some(egress);
        crate::session::shutdown_voice(&joiner, &VoiceShutdownOpts::FULL).await;
        drop(transport);
    }
    assert!(login.is_empty(), "still running: {:?}", login.running());
}
