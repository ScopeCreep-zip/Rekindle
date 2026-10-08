//! Phase 16 — `MockDeps` fixture implementing `VideoDeps` for crate
//! unit tests. Held in-tree (cfg-gated) so any crate test can exercise
//! send/receive paths against deterministic state.

use std::sync::Arc;

use parking_lot::Mutex;
use rekindle_codec::community::envelope::CommunityEnvelope;
use rekindle_secrets::ed25519_dalek::SigningKey;
use rekindle_secrets::media_sender_key::keyring::ChannelSenderKeys;

use crate::deps::{VideoDeps, VideoEvent};
use crate::error::VideoError;

#[derive(Default)]
pub struct MockCalls {
    pub sent: Vec<CommunityEnvelope>,
    pub events: Vec<VideoEvent>,
    pub lamport_calls: u64,
    /// `(community_id, channel_id, sender, index)` per key request.
    pub key_requests: Vec<(String, String, String, u64)>,
}

pub struct MockDeps {
    /// Our channel session's sender keys: the key we send under, and
    /// the keys senders "pushed" to us (tests install them).
    pub keys: Arc<ChannelSenderKeys>,
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
            keys: Arc::new(ChannelSenderKeys::default()),
            signing_key: Some(sk),
            active_channel: Some("11111111111111111111111111111111".to_string()),
            calls: Mutex::new(MockCalls::default()),
            next_lamport: Mutex::new(0),
        }
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
    fn channel_sender_keys(&self, _: &str, _: &str) -> Arc<ChannelSenderKeys> {
        Arc::clone(&self.keys)
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

    fn request_media_key(&self, community_id: &str, channel_id: &str, sender: &str, index: u64) {
        self.calls.lock().key_requests.push((
            community_id.to_string(),
            channel_id.to_string(),
            sender.to_string(),
            index,
        ));
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
}

/// Build a `FrameShape` for tests, with `key_index: 0`.
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
        key_index: 0,
    }
}
