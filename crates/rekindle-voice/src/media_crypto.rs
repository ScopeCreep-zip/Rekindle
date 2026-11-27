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

    /// The SFrame key a frame from `sender_key` under `kid` opens with. In
    /// a channel the KID names only the ratchet step, so each candidate
    /// secret is tried against the frame and the one that opens it is kept
    /// (`ChannelSenderKeys::confirm`).
    fn receive_key(
        &self,
        sender_key: &[u8],
        kid: Kid,
        frame: &[u8],
        metadata: &[u8],
    ) -> Result<SframeKey, OpenError> {
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
                    .map(|m| sframe::media_key(&m.secret, sender_key, kid))
                    .ok_or(OpenError::NoKey { needed_index: None })
            }
            MediaScope::Channel {
                community_id,
                channel_id,
            } => {
                let keys = self.deps.channel_sender_keys(community_id, channel_id);
                let sender = hex::encode(sender_key);
                let candidates = keys
                    .receive_candidates(&sender, sframe::kid_generation_low(kid))
                    .map_err(|missing| OpenError::NoKey {
                        needed_index: Some(missing.index),
                    })?;
                for candidate in candidates {
                    let key = sframe::media_key(&candidate.secret, sender_key, kid);
                    if sframe::open(&key, frame, metadata).is_ok() {
                        keys.confirm(&sender, candidate);
                        return Ok(key);
                    }
                }
                Err(OpenError::Rejected)
            }
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
        let metadata =
            VoicePacket::sframe_metadata(&packet.sender_key, packet.sequence, packet.timestamp);
        let known = self
            .senders
            .get(&packet.sender_key)
            .is_some_and(|states| states.iter().any(|s| s.kid == kid));
        let new_key = if known {
            None
        } else {
            Some(
                self.keys
                    .receive_key(&packet.sender_key, kid, &packet.sframe, &metadata)?,
            )
        };
        let states = self.senders.entry(packet.sender_key.clone()).or_default();
        if let Some(key) = new_key {
            if states.len() == KIDS_PER_SENDER {
                states.remove(0);
            }
            states.push(OpenState {
                kid,
                key,
                window: CtrWindow::new(),
            });
        }
        let Some(state) = states.iter_mut().find(|s| s.kid == kid) else {
            return Err(OpenError::Rejected);
        };
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
#[path = "media_crypto_tests.rs"]
mod tests;
