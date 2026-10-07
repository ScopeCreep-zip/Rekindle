//! SFrame (RFC 9605) sealing and opening of voice frames, shared by the
//! send, receive and MCU loops so every path that touches audio encrypts
//! and decrypts the same way.
//!
//! Keying follows `rekindle_secrets::sframe`: each sender encrypts under
//! its own key, derived from the session's scope secret (the call secret
//! or the channel-media MEK), its signing key and a per-session tag in the
//! KID. The receiver derives the sender's key from the packet's signed
//! `sender_key`, so a frame only opens as the sender who signed it.
//!
//! The SFrame plaintext is `level ‖ opus`: one VAD audio-level byte (plan
//! step E4 fills it; 0 until then) and the Opus frame.

use std::collections::HashMap;
use std::sync::Arc;

use rekindle_secrets::sframe::{self, Kid, SframeKey, SframeSender};
use rekindle_types::channel_keys::{self, ChannelKeyProvider, KeyEpoch, KeyScope, MediaKey};
use rekindle_types::id::ChannelId;
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
    /// No scope secret for the frame's key generation. For a channel,
    /// `needed_generation` names the generation to request.
    NoKey { needed_generation: Option<u64> },
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
                let provider = self.deps.keys();
                let scope = media_key_scope(&*provider, community_id, channel_id)?;
                let (epoch, secret) = channel_keys::current_key(&*provider, community_id, scope)?;
                Some(SendSource {
                    secret,
                    generation: epoch.0,
                    sender: self
                        .deps
                        .channel_media_sender(community_id, channel_id, epoch.0),
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
                    .ok_or(OpenError::NoKey {
                        needed_generation: None,
                    })
            }
            MediaScope::Channel {
                community_id,
                channel_id,
            } => {
                let provider = self.deps.keys();
                let scope = media_key_scope(&*provider, community_id, channel_id)
                    .ok_or(OpenError::Rejected)?;
                let current = provider
                    .current_epoch(community_id, scope)
                    .map_or(0, |epoch| epoch.0);
                let named = generation_named(current, kid);
                match channel_keys::media_key(&*provider, community_id, scope, KeyEpoch(named)) {
                    MediaKey::Key(secret) => Ok(secret),
                    MediaKey::Stale => Err(OpenError::Rejected),
                    MediaKey::Missing { needed } => Err(OpenError::NoKey {
                        needed_generation: Some(needed.0),
                    }),
                }
            }
        }
    }
}

/// The key scope of a community channel's media, or `None` when the
/// channel id is not a channel id.
fn media_key_scope(
    provider: &dyn ChannelKeyProvider,
    community_id: &str,
    channel_id: &str,
) -> Option<KeyScope> {
    ChannelId::from_hex(channel_id).map(|channel| provider.scope_for_media(community_id, channel))
}

/// The full generation a KID's low bits name, relative to our current
/// one: the current generation, the one it replaced, or the next one
/// ahead of us with those low bits (RFC 9605 §5.2: the low-order epoch
/// bits are a window, and a newer epoch with the same bits retires the
/// older).
fn generation_named(current: u64, kid: Kid) -> u64 {
    if current > 0 && sframe::kid_names_generation(kid, current) {
        current
    } else if current > 1 && sframe::kid_names_generation(kid, current - 1) {
        current - 1
    } else {
        next_generation_named(current, kid)
    }
}

/// The first generation after `current` whose low bits the KID carries:
/// a sender ahead of us is on the next generation we have not seen.
fn next_generation_named(current: u64, kid: Kid) -> u64 {
    let low = u64::from(sframe::kid_generation_low(kid));
    let candidate = (current & !0xff) | low;
    if candidate > current {
        candidate
    } else {
        candidate.saturating_add(0x100)
    }
}

/// Our sender state per community voice channel: one per channel, for
/// the channel's current key generation. A new generation gets a fresh
/// state (new tag, CTR from 0); within a generation every session and
/// rebuilt transport continues the same counter (RFC 9605 §9.1).
#[derive(Default)]
pub struct ChannelSframeSenders {
    by_channel: parking_lot::Mutex<HashMap<(String, String), (u64, Arc<SframeSender>)>>,
}

impl ChannelSframeSenders {
    /// The sender state for `(community_id, channel_id)` at `generation`.
    pub fn sender_for(
        &self,
        community_id: &str,
        channel_id: &str,
        generation: u64,
    ) -> Arc<SframeSender> {
        let mut by_channel = self.by_channel.lock();
        let entry = by_channel
            .entry((community_id.to_string(), channel_id.to_string()))
            .or_insert_with(|| (generation, Arc::new(SframeSender::fresh())));
        if entry.0 != generation {
            *entry = (generation, Arc::new(SframeSender::fresh()));
        }
        Arc::clone(&entry.1)
    }

    /// Forget every channel (logout).
    pub fn clear(&self) {
        self.by_channel.lock().clear();
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
        transport_seq: u64,
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
        let metadata = VoicePacket::sframe_metadata(sender_key, sequence, timestamp, transport_seq);
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
        let metadata = VoicePacket::sframe_metadata(
            &packet.sender_key,
            packet.sequence,
            packet.timestamp,
            packet.transport_seq,
        );
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
    use parking_lot::Mutex;

    /// Key source with one call and one channel. `current`/`previous`
    /// are the channel's (MEK, generation); `sender` is our state.
    struct Keys {
        call_secret: [u8; 32],
        call_peer: String,
        call_sender: Arc<SframeSender>,
        channel: Arc<ChannelKeys>,
        channel_senders: Mutex<HashMap<u64, Arc<SframeSender>>>,
    }

    /// The channel's keys: current and the one it replaced, rotated
    /// `age` ago.
    struct ChannelKeys {
        current: Mutex<Option<([u8; 32], u64)>>,
        previous: Mutex<Option<([u8; 32], u64)>>,
        age: Mutex<std::time::Duration>,
    }

    impl ChannelKeyProvider for ChannelKeys {
        fn current_epoch(&self, _: &str, _: KeyScope) -> Option<KeyEpoch> {
            self.current.lock().map(|(_, g)| KeyEpoch(g))
        }
        fn key(&self, _: &str, _: KeyScope, epoch: KeyEpoch) -> Option<Zeroizing<[u8; 32]>> {
            [*self.current.lock(), *self.previous.lock()]
                .into_iter()
                .flatten()
                .find(|(_, g)| *g == epoch.0)
                .map(|(k, _)| Zeroizing::new(k))
        }
        fn current_epoch_age(&self, _: &str, _: KeyScope) -> Option<std::time::Duration> {
            Some(*self.age.lock())
        }
        fn scope_for_text(&self, _: &str, _: ChannelId) -> KeyScope {
            KeyScope::Community
        }
        fn scope_for_media(&self, _: &str, channel: ChannelId) -> KeyScope {
            KeyScope::Channel(channel)
        }
    }

    impl Keys {
        fn new(call_peer: &str) -> Arc<Self> {
            Arc::new(Self {
                call_secret: [7u8; 32],
                call_peer: call_peer.to_string(),
                call_sender: Arc::new(SframeSender::fresh()),
                channel: Arc::new(ChannelKeys {
                    current: Mutex::new(Some(([1u8; 32], 5))),
                    previous: Mutex::new(None),
                    age: Mutex::new(std::time::Duration::ZERO),
                }),
                channel_senders: Mutex::new(HashMap::new()),
            })
        }
    }

    impl MediaKeySource for Keys {
        fn keys(&self) -> Arc<dyn ChannelKeyProvider> {
            self.channel.clone()
        }
        fn channel_media_sender(&self, _: &str, _: &str, generation: u64) -> Arc<SframeSender> {
            Arc::clone(
                self.channel_senders
                    .lock()
                    .entry(generation)
                    .or_insert_with(|| Arc::new(SframeSender::fresh())),
            )
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
        let sframe = sealer.seal(&sender_key, sequence, 1_000, 0, opus).unwrap();
        let mut p = VoicePacket {
            sender_key,
            sequence,
            timestamp: 1_000,
            transport_seq: 0,
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

    #[test]
    fn metadata_is_bound() {
        let alice = SigningKey::from_bytes(&[1u8; 32]);
        let source = Keys::new("unused");
        let mut sealer = FrameSealer::new(keys(&source, channel()));
        let mut opener = FrameOpener::new(keys(&source, channel()));
        let mut p = packet(&alice, &mut sealer, 1, b"opus");
        p.sequence = 2;
        assert_eq!(opener.open(&p), Err(OpenError::Rejected));
    }

    #[test]
    fn channel_generations_current_previous_and_ahead() {
        let alice = SigningKey::from_bytes(&[1u8; 32]);
        let sender_side = Keys::new("unused");
        let mut sealer = FrameSealer::new(keys(&sender_side, channel()));
        let p = packet(&alice, &mut sealer, 1, b"gen5");

        // Receiver on the same generation.
        let same = Keys::new("unused");
        assert_eq!(
            FrameOpener::new(keys(&same, channel())).open(&p).unwrap(),
            b"gen5"
        );

        // Receiver rotated to 6 moments ago, still holding 5.
        let rotated = Keys::new("unused");
        *rotated.channel.current.lock() = Some(([2u8; 32], 6));
        *rotated.channel.previous.lock() = Some(([1u8; 32], 5));
        assert_eq!(
            FrameOpener::new(keys(&rotated, channel()))
                .open(&p)
                .unwrap(),
            b"gen5"
        );

        // Past the grace, the replaced key opens nothing: a removed
        // member's frames under it are refused.
        let settled = Keys::new("unused");
        *settled.channel.current.lock() = Some(([2u8; 32], 6));
        *settled.channel.previous.lock() = Some(([1u8; 32], 5));
        *settled.channel.age.lock() = channel_keys::MEDIA_PREVIOUS_EPOCH_GRACE;
        assert_eq!(
            FrameOpener::new(keys(&settled, channel())).open(&p),
            Err(OpenError::Rejected)
        );

        // Receiver behind on 4: names the generation to request.
        let behind = Keys::new("unused");
        *behind.channel.current.lock() = Some(([9u8; 32], 4));
        assert_eq!(
            FrameOpener::new(keys(&behind, channel())).open(&p),
            Err(OpenError::NoKey {
                needed_generation: Some(5)
            })
        );
    }

    /// A rebuilt transport gets a new sealer, but the sender state lives
    /// in the key source, so the counter continues (RFC 9605 §9.1).
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

    #[test]
    fn channel_senders_continue_within_a_generation_and_renew_across() {
        let senders = ChannelSframeSenders::default();
        let a = senders.sender_for("c", "v", 3);
        assert!(Arc::ptr_eq(&a, &senders.sender_for("c", "v", 3)));
        assert!(!Arc::ptr_eq(&a, &senders.sender_for("c", "other", 3)));
        let renewed = senders.sender_for("c", "v", 4);
        assert!(!Arc::ptr_eq(&a, &renewed));
        assert_ne!(a.kid(4), renewed.kid(4), "a new generation gets a new tag");
    }

    #[test]
    fn next_generation_named_wraps_low_bits() {
        assert_eq!(next_generation_named(5, sframe::media_kid(1, 7)), 7);
        assert_eq!(
            next_generation_named(0x1fe, sframe::media_kid(1, 0x02)),
            0x202
        );
        assert_eq!(next_generation_named(9, sframe::media_kid(1, 9)), 0x109);
    }
}
