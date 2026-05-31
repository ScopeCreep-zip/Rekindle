//! Phase 16 — `MockDeps` fixture implementing `VideoDeps` for crate
//! unit tests. Held in-tree (cfg-gated) so any crate test can exercise
//! send/receive paths against deterministic state.

#![cfg(test)]

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
}

pub struct MockDeps {
    pub mek: Option<MediaEncryptionKey>,
    pub signing_key: Option<SigningKey>,
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
}

impl VideoDeps for MockDeps {
    fn community_mek_bytes(&self, _c: &str) -> Option<([u8; 32], u64)> {
        self.mek.as_ref().map(|m| (*m.as_bytes(), m.generation()))
    }

    fn community_signing_key(&self, _c: &str) -> Option<SigningKey> {
        self.signing_key.clone()
    }

    fn send_to_mesh(&self, _c: &str, envelope: &CommunityEnvelope) -> Result<(), VideoError> {
        self.calls.lock().sent.push(envelope.clone());
        Ok(())
    }

    fn increment_lamport(&self, _c: &str) -> u64 {
        let mut next = self.next_lamport.lock();
        *next += 1;
        self.calls.lock().lamport_calls += 1;
        *next
    }

    fn emit_event(&self, event: VideoEvent) {
        self.calls.lock().events.push(event);
    }
}
