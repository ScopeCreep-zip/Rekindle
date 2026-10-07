//! Phase 16 — `MockDeps` fixture implementing `VideoDeps` for crate
//! unit tests. Held in-tree (cfg-gated) so any crate test can exercise
//! send/receive paths against deterministic state.

use parking_lot::Mutex;
use rekindle_crypto::group::media_key::MediaEncryptionKey;
use rekindle_protocol::dht::community::envelope::CommunityEnvelope;
use rekindle_secrets::ed25519_dalek::SigningKey;

use crate::deps::{VideoDeps, VideoEvent};
use crate::error::VideoError;

#[derive(Default)]
pub struct MockCalls {
    pub sent: Vec<CommunityEnvelope>,
    pub events: Vec<VideoEvent>,
    pub lamport_calls: u64,
    pub mek_refresh_requests: Vec<(String, String)>,
    /// `(community_id, channel_id, last_frame_seq, kbps, loss_q8)` for
    /// each receiver `FrameAck` the receive path emitted.
    pub frame_acks: Vec<(String, String, u32, u32, u8)>,
}

pub struct MockDeps {
    pub mek: Option<MediaEncryptionKey>,
    pub signing_key: Option<SigningKey>,
    /// Channel the mock reports as the local active voice/video
    /// session for every community. Tests default to `"11111111111111111111111111111111"`; the
    /// send-path smoke tests address other channels but never hit the
    /// receive gate.
    pub active_channel: Option<String>,
    pub calls: Mutex<MockCalls>,
    pub next_lamport: Mutex<u64>,
}

impl MockDeps {
    pub fn new() -> Self {
        // Deterministic signing key for tests
        let sk = SigningKey::from_bytes(&[7u8; 32]);
        Self {
            mek: Some(MediaEncryptionKey::from_bytes([1u8; 32], 1)),
            signing_key: Some(sk),
            active_channel: Some("11111111111111111111111111111111".to_string()),
            calls: Mutex::new(MockCalls::default()),
            next_lamport: Mutex::new(0),
        }
    }

    pub fn without_mek() -> Self {
        let mut me = Self::new();
        me.mek = None;
        me
    }

    pub fn without_signing_key() -> Self {
        let mut me = Self::new();
        me.signing_key = None;
        me
    }

    pub fn in_channel(channel: Option<&str>) -> Self {
        let mut me = Self::new();
        me.active_channel = channel.map(str::to_string);
        me
    }
}

impl VideoDeps for MockDeps {
    fn keys(&self) -> std::sync::Arc<dyn rekindle_types::channel_keys::ChannelKeyProvider> {
        std::sync::Arc::new(MockKeys(self.mek.clone()))
    }

    fn community_signing_key(&self, _c: &str) -> Option<SigningKey> {
        self.signing_key.clone()
    }

    fn send_to_channel(
        &self,
        _c: &str,
        _channel_id: &str,
        envelope: &CommunityEnvelope,
    ) -> Result<(), VideoError> {
        self.calls.lock().sent.push(envelope.clone());
        Ok(())
    }

    fn local_active_channel(&self, _c: &str) -> Option<String> {
        self.active_channel.clone()
    }

    fn request_mek_refresh(&self, community_id: &str, channel_id: &str, _needed_generation: u64) {
        self.calls
            .lock()
            .mek_refresh_requests
            .push((community_id.to_string(), channel_id.to_string()));
    }

    fn increment_lamport(&self, _c: &str) -> Result<u64, rekindle_types::lamport::LamportError> {
        let mut next = self.next_lamport.lock();
        *next += 1;
        self.calls.lock().lamport_calls += 1;
        Ok(*next)
    }

    fn emit_event(&self, event: VideoEvent) {
        self.calls.lock().events.push(event);
    }

    fn send_frame_ack(
        &self,
        community_id: &str,
        channel_id: &str,
        _stream_id: [u8; 16],
        last_frame_seq: u32,
        kbps: u32,
        loss_q8: u8,
    ) {
        self.calls.lock().frame_acks.push((
            community_id.to_string(),
            channel_id.to_string(),
            last_frame_seq,
            kbps,
            loss_q8,
        ));
    }
}

/// Build a `FrameShape` for tests, with `mek_generation: 0`.
///
/// `fragment/tests.rs` and `reassembler/tests.rs` each carried a
/// byte-identical copy of this — a duplicate introduced when those two
/// modules were split into sibling test files. One copy, in the module
/// that already exists to hold crate-wide test fixtures.
pub(crate) fn test_shape(
    stream_id: [u8; crate::fragment::STREAM_ID_LEN],
    frame_seq: u32,
    keyframe: bool,
    codec: rekindle_types::video::Codec,
    timestamp: u32,
) -> crate::fragment::FrameShape {
    crate::fragment::FrameShape {
        stream_id,
        frame_seq,
        keyframe,
        codec,
        timestamp,
        mek_generation: 0,
    }
}

/// The mock's one channel key, under every channel's media scope.
struct MockKeys(Option<MediaEncryptionKey>);

impl rekindle_types::channel_keys::ChannelKeyProvider for MockKeys {
    fn current_epoch(
        &self,
        _: &str,
        _: rekindle_types::channel_keys::KeyScope,
    ) -> Option<rekindle_types::channel_keys::KeyEpoch> {
        self.0
            .as_ref()
            .map(|m| rekindle_types::channel_keys::KeyEpoch(m.generation()))
    }

    fn key(
        &self,
        _: &str,
        _: rekindle_types::channel_keys::KeyScope,
        epoch: rekindle_types::channel_keys::KeyEpoch,
    ) -> Option<rekindle_types::channel_keys::Zeroizing<[u8; 32]>> {
        self.0
            .as_ref()
            .filter(|m| m.generation() == epoch.0)
            .map(|m| rekindle_types::channel_keys::Zeroizing::new(*m.as_bytes()))
    }

    fn current_epoch_age(
        &self,
        _: &str,
        _: rekindle_types::channel_keys::KeyScope,
    ) -> Option<std::time::Duration> {
        self.0.as_ref().map(|_| std::time::Duration::from_secs(60))
    }

    fn scope_for_text(
        &self,
        _: &str,
        _: rekindle_types::id::ChannelId,
    ) -> rekindle_types::channel_keys::KeyScope {
        rekindle_types::channel_keys::KeyScope::Community
    }

    fn scope_for_media(
        &self,
        _: &str,
        channel: rekindle_types::id::ChannelId,
    ) -> rekindle_types::channel_keys::KeyScope {
        rekindle_types::channel_keys::KeyScope::Channel(channel)
    }
}
