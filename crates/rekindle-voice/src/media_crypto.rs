//! SFrame (RFC 9605) sealing and opening of voice frames, shared by the
//! send, receive and MCU loops so every path that touches audio encrypts
//! and decrypts the same way.
//!
//! Keying follows `rekindle_secrets::sframe`: each sender encrypts under
//! its own key, derived from a 32-byte secret, its signing key and a
//! per-session tag in the KID. The receiver derives the sender's key from
//! the packet's signed `sender_key`, so a frame only opens as the sender
//! who signed it. The secret is the call secret in a 1:1 call; in a
//! community channel it is the sender's own media key, which only that
//! sender encrypts under (`rekindle_secrets::media_sender_key::keyring`, plan C7.20).
//!
//! The SFrame plaintext is `level ‖ opus`: one VAD audio-level byte (plan
//! step E4 fills it; 0 until then) and the Opus frame.

use std::collections::HashMap;
use std::sync::Arc;

use rekindle_secrets::sframe::{self, Kid, SframeKey, SframeSender};
use zeroize::Zeroizing;

use crate::replay_window::CtrWindow;
use crate::session_deps::MediaKeySource;
use crate::transport::VoicePacket;

/// What a voice session's media is keyed under.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaScope {
    /// A 1:1 call with `peer` (identity key hex).
    Call { peer: String },
    /// A community voice channel.
    Channel {
        community_id: String,
        channel_id: String,
    },
}

impl MediaScope {
    /// The scope of a session on `channel_id`: a community channel when
    /// `community_id` is set, otherwise a 1:1 call whose channel id is the
    /// peer's identity key.
    #[must_use]
    pub fn of_session(community_id: Option<&str>, channel_id: &str) -> Self {
        match community_id {
            Some(community_id) => Self::Channel {
                community_id: community_id.to_string(),
                channel_id: channel_id.to_string(),
            },
            None => Self::Call {
                peer: channel_id.to_string(),
            },
        }
    }
}

/// Why a frame was not opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenError {
    /// No key for the frame. For a channel, `needed_index` names the
    /// sender's key index to request.
    NoKey { needed_index: Option<u64> },
    /// In a 1:1 call, a frame from someone other than the call's peer.
    WrongSender,
    /// Malformed, unauthenticated or replayed. Callers count these and
    /// do not log them one by one (RFC 9605 §4.4.4).
    Rejected,
}

/// The key sources of one session scope.
pub struct MediaKeys {
    deps: Arc<dyn MediaKeySource>,
    scope: MediaScope,
}

/// The secret, generation and sender state a sender seals with.
struct SendSource {
    secret: Zeroizing<[u8; 32]>,
    generation: u64,
    sender: Arc<SframeSender>,
}

impl MediaKeys {
    #[must_use]
    pub fn new(deps: Arc<dyn MediaKeySource>, scope: MediaScope) -> Self {
        Self { deps, scope }
    }

    #[must_use]
    pub fn scope(&self) -> &MediaScope {
        &self.scope
    }

    fn send_source(&self) -> Option<SendSource> {
        match &self.scope {
            MediaScope::Call { peer } => {
                let media = self.deps.call_media(peer)?;
                Some(SendSource {
                    secret: media.secret,
                    generation: 0,
                    sender: media.sender,
                })
            }
            MediaScope::Channel {
                community_id,
                channel_id,
            } => {
                let key = self
                    .deps
                    .channel_sender_keys(community_id, channel_id)
                    .send_key(std::time::Instant::now());
                Some(SendSource {
                    secret: key.secret,
                    generation: key.index,
                    sender: key.sender,
                })
            }
        }
    }

    /// The scope secret for a frame from `sender_key` under `kid`.
    fn receive_secret(
        &self,
        sender_key: &[u8],
        kid: Kid,
    ) -> Result<Zeroizing<[u8; 32]>, OpenError> {
        match &self.scope {
            MediaScope::Call { peer } => {
                if hex::encode(sender_key) != *peer {
                    return Err(OpenError::WrongSender);
                }
                if !sframe::kid_names_generation(kid, 0) {
                    return Err(OpenError::Rejected);
                }
                self.deps
                    .call_media(peer)
                    .map(|m| m.secret)
                    .ok_or(OpenError::NoKey { needed_index: None })
            }
            MediaScope::Channel {
                community_id,
                channel_id,
            } => self
                .deps
                .channel_sender_keys(community_id, channel_id)
                .receive_secret(&hex::encode(sender_key), sframe::kid_generation_low(kid))
                .map_err(|missing| OpenError::NoKey {
                    needed_index: Some(missing.index),
                }),
        }
    }
}

/// Seals our outbound frames. Derives the SFrame key once per signing
/// key, sender state and generation.
pub struct FrameSealer {
    keys: Arc<MediaKeys>,
    cached: Option<CachedSendKey>,
}

struct CachedSendKey {
    sender_key: Vec<u8>,
    generation: u64,
    sender: Arc<SframeSender>,
    kid: Kid,
    key: SframeKey,
}

impl FrameSealer {
    #[must_use]
    pub fn new(keys: Arc<MediaKeys>) -> Self {
        Self { keys, cached: None }
    }

    /// The SFrame ciphertext of `opus` for a packet signed by
    /// `sender_key` with these fields, or `None` when the scope has no key
    /// yet — the frame is then dropped, never sent in the clear.
    pub fn seal(
        &mut self,
        sender_key: &[u8],
        sequence: u32,
        timestamp: u64,
        opus: &[u8],
    ) -> Option<Vec<u8>> {
        let source = self.keys.send_source()?;
        let reuse = self.cached.as_ref().is_some_and(|c| {
            c.sender_key == sender_key
                && c.generation == source.generation
                && Arc::ptr_eq(&c.sender, &source.sender)
        });
        if !reuse {
            let kid = source.sender.kid(source.generation);
            self.cached = Some(CachedSendKey {
                sender_key: sender_key.to_vec(),
                generation: source.generation,
                kid,
                key: sframe::media_key(&source.secret, sender_key, kid),
                sender: source.sender,
            });
        }
        let cached = self.cached.as_ref()?;
        let metadata = VoicePacket::sframe_metadata(sender_key, sequence, timestamp);
        let mut plaintext = Vec::with_capacity(1 + opus.len());
        plaintext.push(0);
        plaintext.extend_from_slice(opus);
        sframe::seal(
            &cached.key,
            cached.kid,
            cached.sender.next_ctr(),
            &metadata,
            &plaintext,
        )
        .ok()
    }
}

/// Keys and replay windows a sender has used recently; more than this and
/// the oldest is forgotten.
const KIDS_PER_SENDER: usize = 4;

/// Opens inbound frames: per sender, the derived keys of its recent KIDs
/// and a CTR replay window for each. Kept for the whole session — a
/// VAD-silent participant timing out of the mixer must not reset its
/// replay window.
pub struct FrameOpener {
    keys: Arc<MediaKeys>,
    senders: HashMap<Vec<u8>, Vec<OpenState>>,
}

struct OpenState {
    kid: Kid,
    key: SframeKey,
    window: CtrWindow,
}

impl FrameOpener {
    #[must_use]
    pub fn new(keys: Arc<MediaKeys>) -> Self {
        Self {
            keys,
            senders: HashMap::new(),
        }
    }

    /// The Opus frame a verified packet carries.
    pub fn open(&mut self, packet: &VoicePacket) -> Result<Vec<u8>, OpenError> {
        let (kid, ctr, _) =
            sframe::parse_header(&packet.sframe).map_err(|_| OpenError::Rejected)?;
        let states = self.senders.entry(packet.sender_key.clone()).or_default();
        let index = if let Some(i) = states.iter().position(|s| s.kid == kid) {
            i
        } else {
            let secret = self.keys.receive_secret(&packet.sender_key, kid)?;
            if states.len() == KIDS_PER_SENDER {
                states.remove(0);
            }
            states.push(OpenState {
                kid,
                key: sframe::media_key(&secret, &packet.sender_key, kid),
                window: CtrWindow::new(),
            });
            states.len() - 1
        };
        let state = &mut states[index];
        let metadata =
            VoicePacket::sframe_metadata(&packet.sender_key, packet.sequence, packet.timestamp);
        let plaintext =
            sframe::open(&state.key, &packet.sframe, &metadata).map_err(|_| OpenError::Rejected)?;
        // Replay is checked after authentication, so forged frames can't
        // advance the window.
        if !state.window.check_and_insert(ctr) {
            return Err(OpenError::Rejected);
        }
        match plaintext.split_first() {
            Some((_level, opus)) => Ok(opus.to_vec()),
            None => Err(OpenError::Rejected),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session_deps::CallMediaKeys;
    use ed25519_dalek::SigningKey;
    use rekindle_codec::capnp_codec::SignedWire;
    use rekindle_secrets::media_sender_key::keyring::ChannelSenderKeys;

    /// One side's key source: a call with `call_peer`, and one channel
    /// session's sender keys.
    struct Keys {
        call_secret: [u8; 32],
        call_peer: String,
        call_sender: Arc<SframeSender>,
        channel: Arc<ChannelSenderKeys>,
    }

    impl Keys {
        fn new(call_peer: &str) -> Arc<Self> {
            Arc::new(Self {
                call_secret: [7u8; 32],
                call_peer: call_peer.to_string(),
                call_sender: Arc::new(SframeSender::fresh()),
                channel: Arc::new(ChannelSenderKeys::default()),
            })
        }
    }

    impl MediaKeySource for Keys {
        fn channel_sender_keys(&self, _: &str, _: &str) -> Arc<ChannelSenderKeys> {
            Arc::clone(&self.channel)
        }
        fn call_media(&self, peer: &str) -> Option<CallMediaKeys> {
            (peer == self.call_peer).then(|| CallMediaKeys {
                secret: Zeroizing::new(self.call_secret),
                sender: Arc::clone(&self.call_sender),
            })
        }
    }

    fn channel() -> MediaScope {
        MediaScope::Channel {
            community_id: "c".into(),
            channel_id: "0a".repeat(16),
        }
    }

    fn packet(
        signer: &SigningKey,
        sealer: &mut FrameSealer,
        sequence: u32,
        opus: &[u8],
    ) -> VoicePacket {
        let sender_key = signer.verifying_key().to_bytes().to_vec();
        let sframe = sealer.seal(&sender_key, sequence, 1_000, opus).unwrap();
        let mut p = VoicePacket {
            sender_key,
            sequence,
            timestamp: 1_000,
            sframe,
            sig: Vec::new(),
        };
        p.sign(signer);
        p
    }

    fn keys(source: &Arc<Keys>, scope: MediaScope) -> Arc<MediaKeys> {
        let source: Arc<dyn MediaKeySource> = source.clone();
        Arc::new(MediaKeys::new(source, scope))
    }

    /// Give `receiver` the key `sender` sends under now, as a pushed key.
    fn share(sender: &Arc<Keys>, sender_hex: &str, receiver: &Arc<Keys>) {
        let key = sender.channel.send_key(std::time::Instant::now());
        receiver.channel.install(sender_hex, key.index, key.secret);
    }

    #[test]
    fn call_frames_open_only_from_the_peer_and_only_once() {
        let alice = SigningKey::from_bytes(&[1u8; 32]);
        let alice_hex = hex::encode(alice.verifying_key().to_bytes());
        let bob_hex = "bb".repeat(32);
        // Each side's scope names the other; both hold the call secret.
        let alice_keys = Keys::new(&bob_hex);
        let bob_keys = Keys::new(&alice_hex);
        let mut sealer = FrameSealer::new(keys(&alice_keys, MediaScope::Call { peer: bob_hex }));
        let mut opener = FrameOpener::new(keys(&bob_keys, MediaScope::Call { peer: alice_hex }));

        let p = packet(&alice, &mut sealer, 1, b"opus");
        assert_eq!(opener.open(&p).unwrap(), b"opus");
        assert_eq!(opener.open(&p), Err(OpenError::Rejected), "replay");

        let mallory = SigningKey::from_bytes(&[2u8; 32]);
        let forged = packet(&mallory, &mut sealer, 2, b"x");
        assert_eq!(opener.open(&forged), Err(OpenError::WrongSender));
    }

    /// A channel frame opens with the sender's own key once it was pushed
    /// to us, and names the index to ask for before then.
    #[test]
    fn channel_frames_open_under_the_senders_own_key() {
        let alice = SigningKey::from_bytes(&[1u8; 32]);
        let alice_hex = hex::encode(alice.verifying_key().to_bytes());
        let alice_side = Keys::new("unused");
        let bob_side = Keys::new("unused");
        let mut sealer = FrameSealer::new(keys(&alice_side, channel()));
        let p = packet(&alice, &mut sealer, 1, b"hello");

        let mut opener = FrameOpener::new(keys(&bob_side, channel()));
        let index = alice_side.channel.send_key(std::time::Instant::now()).index;
        assert_eq!(
            opener.open(&p),
            Err(OpenError::NoKey {
                needed_index: Some(index & 0xff)
            })
        );
        share(&alice_side, &alice_hex, &bob_side);
        assert_eq!(opener.open(&p).unwrap(), b"hello");
    }

    /// Another member's key never opens Alice's frames: each sender's key
    /// has one writer, so no two members can disagree on it.
    #[test]
    fn a_frame_does_not_open_under_another_senders_key() {
        let alice = SigningKey::from_bytes(&[1u8; 32]);
        let alice_hex = hex::encode(alice.verifying_key().to_bytes());
        let alice_side = Keys::new("unused");
        let carol_side = Keys::new("unused");
        let bob_side = Keys::new("unused");
        let mut sealer = FrameSealer::new(keys(&alice_side, channel()));
        let p = packet(&alice, &mut sealer, 1, b"x");
        // Bob holds Carol's key filed under Alice: it does not open.
        share(&carol_side, &alice_hex, &bob_side);
        let index = alice_side.channel.send_key(std::time::Instant::now()).index;
        let carol_index = carol_side.channel.send_key(std::time::Instant::now()).index;
        let result = FrameOpener::new(keys(&bob_side, channel())).open(&p);
        if index & 0xff == carol_index & 0xff {
            assert_eq!(result, Err(OpenError::Rejected));
        } else {
            assert!(matches!(result, Err(OpenError::NoKey { .. })));
        }
    }

    #[test]
    fn metadata_is_bound() {
        let alice = SigningKey::from_bytes(&[1u8; 32]);
        let alice_hex = hex::encode(alice.verifying_key().to_bytes());
        let source = Keys::new("unused");
        let mut sealer = FrameSealer::new(keys(&source, channel()));
        share(&source, &alice_hex, &source);
        let mut opener = FrameOpener::new(keys(&source, channel()));
        let mut p = packet(&alice, &mut sealer, 1, b"opus");
        p.sequence = 2;
        assert_eq!(opener.open(&p), Err(OpenError::Rejected));
    }

    /// A rebuilt transport gets a new sealer, but the sender state lives
    /// in the session's keys, so the counter continues (RFC 9605 §9.1).
    #[test]
    fn counter_continues_across_sealer_rebuilds() {
        let source = Keys::new("unused");
        let alice = SigningKey::from_bytes(&[1u8; 32]);
        let mut first = FrameSealer::new(keys(&source, channel()));
        let a = packet(&alice, &mut first, 1, b"a");
        let mut second = FrameSealer::new(keys(&source, channel()));
        let b = packet(&alice, &mut second, 2, b"b");
        let (kid_a, ctr_a, _) = sframe::parse_header(&a.sframe).unwrap();
        let (kid_b, ctr_b, _) = sframe::parse_header(&b.sframe).unwrap();
        assert_eq!(kid_a, kid_b);
        assert_ne!(ctr_a, ctr_b);
    }
}
