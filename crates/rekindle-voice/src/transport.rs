use std::collections::HashMap;

use chacha20poly1305::{
    aead::{Aead, KeyInit},
    ChaCha20Poly1305, Key, Nonce,
};
use rekindle_route::contexts::RouteContextSpec;
use serde::{Deserialize, Serialize};
use veilid_core::{RoutingContext, SafetySelection, Sequencing, Target, VeilidAPI};

use crate::codec::EncodedFrame;
use crate::error::VoiceError;

// Wave 13 W13.14 — AEAD audio encryption under the X25519-derived
// call_key. ChaCha20-Poly1305 chosen for low-CPU (matters for mobile),
// constant-time (no timing oracles), large nonce space (12 bytes —
// no birthday attack at audio packet rates), and a Rust ecosystem
// implementation already used elsewhere in the project's crypto
// dependencies.

/// Domain-tag bytes that go into the nonce derivation so a chat or
/// governance ChaCha20-Poly1305 secret can never collide with a voice
/// nonce reused (defense-in-depth — the call_key is already
/// domain-separated by `derive_call_key`'s HKDF info).
const VOICE_AEAD_DOMAIN: &[u8; 4] = b"vca1";

/// Build a 12-byte nonce from `(sequence, timestamp)`. Each packet
/// gets a unique nonce because the (sequence, timestamp) pair is
/// strictly monotonic per call. Sender and receiver reconstruct the
/// same nonce from the public packet fields — no extra wire bytes.
fn aead_nonce(sequence: u32, timestamp: u64) -> [u8; 12] {
    // 4-byte domain tag + 4-byte sequence + 4 low-order bytes of
    // timestamp. (sequence, timestamp) is monotonic per call so the
    // resulting 12-byte nonce is unique even if timestamps roll over
    // every ~50 days at 48 kHz Opus framing — at which point sequence
    // alone is 32 bits of fresh space.
    let mut nonce = [0u8; 12];
    nonce[..4].copy_from_slice(VOICE_AEAD_DOMAIN);
    nonce[4..8].copy_from_slice(&sequence.to_le_bytes());
    nonce[8..12].copy_from_slice(&((timestamp & 0xFFFF_FFFF) as u32).to_le_bytes());
    nonce
}

fn encrypt_audio(
    call_key: &[u8; 32],
    sequence: u32,
    timestamp: u64,
    plaintext: &[u8],
) -> Result<Vec<u8>, VoiceError> {
    let cipher = ChaCha20Poly1305::new(Key::from_slice(call_key));
    let nonce = aead_nonce(sequence, timestamp);
    cipher
        .encrypt(Nonce::from_slice(&nonce), plaintext)
        .map_err(|e| VoiceError::Transport(format!("aead encrypt: {e}")))
}

/// Receive-side decrypt entry point. Made public so receive_loop can
/// run the decrypt step after signature verification.
pub fn decrypt_packet_audio(
    call_key: &[u8; 32],
    packet: &VoicePacket,
) -> Result<Vec<u8>, VoiceError> {
    let cipher = ChaCha20Poly1305::new(Key::from_slice(call_key));
    let nonce = aead_nonce(packet.sequence, packet.timestamp);
    cipher
        .decrypt(Nonce::from_slice(&nonce), packet.audio_data.as_slice())
        .map_err(|e| VoiceError::Transport(format!("aead decrypt: {e}")))
}

/// Voice packet for network transmission.
///
/// Architecture §10.3 + §26 W26 — `signature` is an Ed25519 signature
/// by the sender's pseudonym secret over [`signing_bytes`]. Receivers
/// MUST verify against `sender_key` before mixing/playing the audio,
/// otherwise any community member could MEK-encrypt audio claiming to
/// be any other (the MEK is community-shared and authenticates only
/// "some member encrypted this", not "this specific member").
#[derive(Clone, Serialize, Deserialize)]
pub struct VoicePacket {
    /// Sender public key (32 bytes).
    pub sender_key: Vec<u8>,
    /// Sequence number for ordering.
    pub sequence: u32,
    /// Timestamp in milliseconds.
    pub timestamp: u64,
    /// Opus-encoded audio data (MEK-encrypted ciphertext on the wire
    /// for community channels; plaintext for 1:1 calls).
    pub audio_data: Vec<u8>,
    /// 64-byte Ed25519 signature over [`signing_bytes`]. Receivers
    /// reject packets with an empty or invalid signature.
    #[serde(default)]
    pub signature: Vec<u8>,
}

impl VoicePacket {
    /// Canonical bytes the sender signs. Domain-tagged so a signature
    /// for a chat or governance subkey can't be replayed as a voice
    /// packet (and vice versa).
    pub fn signing_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(
            b"rekindle-voice-packet-v1".len()
                + self.sender_key.len()
                + 12
                + self.audio_data.len(),
        );
        out.extend_from_slice(b"rekindle-voice-packet-v1");
        out.extend_from_slice(&self.sender_key);
        out.extend_from_slice(&self.sequence.to_le_bytes());
        out.extend_from_slice(&self.timestamp.to_le_bytes());
        out.extend_from_slice(&self.audio_data);
        out
    }
}

/// Voice channel operating mode.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum VoiceMode {
    /// Full mesh: each participant sends to every other.
    /// Suitable for 2-5 participants.
    #[default]
    Mesh,
    /// MCU mode: one peer (host) receives all, mixes, redistributes.
    /// Suitable for 6-15+ participants.
    Mcu {
        /// Pseudonym key (hex) of the mixing host.
        host_pseudonym: String,
    },
}

/// Per-peer routing info for multi-peer voice transport.
struct PeerRoute {
    routing_context: RoutingContext,
    route_id: veilid_core::RouteId,
}

/// Multi-peer voice transport over the Veilid network.
///
/// Supports both full-mesh (each peer sends to every other) and MCU
/// (all peers send to one host, host mixes and redistributes) modes.
///
/// Uses `SafetySelection::Unsafe` for voice to minimize latency,
/// trading sender privacy for acceptable voice quality.
pub struct VoiceTransport {
    channel_id: String,
    api: Option<VeilidAPI>,
    sender_key: Vec<u8>,
    /// Architecture §10.3 + §26 W26 — pseudonym signing key for voice
    /// packet authentication. Derived from the user's identity secret +
    /// community_id (or identity secret alone for 1:1 calls). Cleared on
    /// disconnect so a stale key can't sign packets after channel exit.
    signing_key: Option<ed25519_dalek::SigningKey>,
    /// Wave 13 W13.14 — optional AEAD key for audio encryption.
    /// `Some(call_key)` for 1:1 DM calls (X25519 ECDH derived). For
    /// community voice, the per-channel MEK is applied at a higher
    /// layer in `services/voice/send_loop.rs`, so this stays None and
    /// audio_data passes through unmodified.
    call_key: Option<[u8; 32]>,
    /// Connected peers: pseudonym_key (hex) → route info.
    peers: HashMap<String, PeerRoute>,
    /// Current operating mode.
    mode: VoiceMode,
}

impl VoiceTransport {
    fn build_voice_routing_context(api: &VeilidAPI) -> Result<RoutingContext, VoiceError> {
        let spec = RouteContextSpec::rc_voice();
        api.routing_context()
            .map_err(|e| VoiceError::Transport(format!("routing context: {e}")))?
            .with_safety(match spec.kind {
                rekindle_route::contexts::RouteContextKind::Voice => {
                    SafetySelection::Unsafe(Sequencing::NoPreference)
                }
                rekindle_route::contexts::RouteContextKind::Safe => {
                    SafetySelection::Unsafe(Sequencing::PreferOrdered)
                }
            })
            .map_err(|e| VoiceError::Transport(format!("with_safety: {e}")))
    }

    /// Create a new transport for a voice channel.
    pub fn new(channel_id: String) -> Self {
        Self {
            channel_id,
            api: None,
            sender_key: Vec::new(),
            signing_key: None,
            call_key: None,
            peers: HashMap::new(),
            mode: VoiceMode::default(),
        }
    }

    /// Wave 13 W13.14 — install the AEAD call_key for 1:1 DM calls so
    /// every outbound packet's `audio_data` is ChaCha20-Poly1305
    /// encrypted under the X25519-ECDH-derived shared key.
    /// Architecture §10.10 mandate. Receivers verify the signature
    /// against the (encrypted) audio_data, then decrypt.
    pub fn set_call_key(&mut self, call_key: [u8; 32]) {
        self.call_key = Some(call_key);
    }

    /// Get the installed call_key (for the receive path which decrypts
    /// after signature verify). Returns None for community voice or
    /// when no call has set up a key yet.
    pub fn call_key(&self) -> Option<[u8; 32]> {
        self.call_key
    }

    /// Initialize the transport with a Veilid API and sender identity.
    ///
    /// This replaces the old single-peer `connect()` for backward compatibility.
    /// After calling `init()`, add peers with `add_peer()`.
    pub fn init(&mut self, api: VeilidAPI, sender_key: Vec<u8>) {
        self.api = Some(api);
        self.sender_key = sender_key;
    }

    /// Architecture §26 W26 — install the pseudonym signing key the
    /// transport uses to sign every outbound voice packet. Caller is
    /// responsible for re-installing on community switch (the key is
    /// derived per-community).
    pub fn set_signing_key(&mut self, signing_key: ed25519_dalek::SigningKey) {
        self.signing_key = Some(signing_key);
    }

    /// Legacy single-peer connect (wraps `init` + `add_peer`).
    ///
    /// Kept for backward compatibility with 1:1 DM voice calls.
    /// Uses `"default"` as the peer key for the single remote participant.
    pub fn connect(
        &mut self,
        api: VeilidAPI,
        route_blob: &[u8],
        sender_key: Vec<u8>,
    ) -> Result<(), VoiceError> {
        self.init(api, sender_key);
        self.add_peer("default", route_blob)?;
        tracing::info!(channel = %self.channel_id, "voice transport connected (legacy single-peer)");
        Ok(())
    }

    /// Add a peer to the voice mesh.
    pub fn add_peer(&mut self, pseudonym_key: &str, route_blob: &[u8]) -> Result<(), VoiceError> {
        let api = self
            .api
            .as_ref()
            .ok_or_else(|| VoiceError::Transport("transport not initialized".into()))?;

        let routing_context = Self::build_voice_routing_context(api)?;

        let route_id = api
            .import_remote_private_route(route_blob.to_vec())
            .map_err(|e| VoiceError::Transport(format!("import route: {e}")))?;

        tracing::info!(
            channel = %self.channel_id,
            peer = %pseudonym_key,
            "added voice peer"
        );

        self.peers.insert(
            pseudonym_key.to_string(),
            PeerRoute {
                routing_context,
                route_id,
            },
        );

        Ok(())
    }

    /// Remove a peer from the voice mesh.
    pub fn remove_peer(&mut self, pseudonym_key: &str) {
        if self.peers.remove(pseudonym_key).is_some() {
            tracing::info!(
                channel = %self.channel_id,
                peer = %pseudonym_key,
                "removed voice peer"
            );
        }
    }

    /// Set the voice channel operating mode.
    pub fn set_mode(&mut self, mode: VoiceMode) {
        tracing::info!(channel = %self.channel_id, ?mode, "voice mode changed");
        self.mode = mode;
    }

    /// Get the current voice mode.
    pub fn mode(&self) -> &VoiceMode {
        &self.mode
    }

    /// Number of connected peers.
    pub fn peer_count(&self) -> usize {
        self.peers.len()
    }

    /// Returns the pseudonym keys of all connected peers.
    pub fn peer_keys(&self) -> Vec<String> {
        self.peers.keys().cloned().collect()
    }

    /// Broadcast an encoded audio frame to ALL connected peers (mesh mode).
    ///
    /// Returns a list of (pseudonym_key, error) for any failed sends.
    pub async fn broadcast(&self, frame: &EncodedFrame) -> Vec<(String, VoiceError)> {
        let data = match self.build_packet_data(frame) {
            Ok(d) => d,
            Err(e) => return vec![("*".into(), e)],
        };

        let mut errors = Vec::new();
        for (key, peer) in &self.peers {
            if let Err(e) = peer
                .routing_context
                .app_message(Target::RouteId(peer.route_id.clone()), data.clone())
                .await
            {
                errors.push((
                    key.clone(),
                    VoiceError::Transport(format!("app_message: {e}")),
                ));
            }
        }
        errors
    }

    /// Send an encoded audio frame to a specific peer (MCU mode).
    pub async fn send_to_peer(
        &self,
        pseudonym_key: &str,
        frame: &EncodedFrame,
    ) -> Result<(), VoiceError> {
        let peer = self
            .peers
            .get(pseudonym_key)
            .ok_or_else(|| VoiceError::Transport(format!("peer not found: {pseudonym_key}")))?;

        let data = self.build_packet_data(frame)?;

        peer.routing_context
            .app_message(Target::RouteId(peer.route_id.clone()), data)
            .await
            .map_err(|e| VoiceError::Transport(format!("app_message: {e}")))?;

        Ok(())
    }

    /// Legacy single-peer send (broadcasts to all peers).
    ///
    /// Kept for backward compatibility — the send loop calls this.
    pub async fn send(&self, frame: &EncodedFrame) -> Result<(), VoiceError> {
        if self.peers.is_empty() {
            return Err(VoiceError::NotConnected);
        }

        let errors = self.broadcast(frame).await;
        if errors.len() == self.peers.len() {
            // All sends failed — report the first error
            return Err(errors
                .into_iter()
                .next()
                .map_or(VoiceError::NotConnected, |(_, e)| e));
        }
        Ok(())
    }

    /// Disconnect from the voice channel — removes all peers.
    pub fn disconnect(&mut self) {
        self.peers.clear();
        self.api = None;
        self.sender_key.clear();
        self.signing_key = None;
        self.call_key = None;
        self.mode = VoiceMode::default();
        tracing::info!(channel = %self.channel_id, "voice transport disconnected");
    }

    /// Deserialize an incoming voice packet from raw bytes and verify
    /// its Ed25519 signature against `sender_key`. Architecture §10.3 +
    /// §26 W26 — packets with missing or invalid signatures are
    /// rejected (returns `VoiceError::Transport`).
    ///
    /// Expects data WITHOUT the `b'V'` type tag prefix — the dispatch loop
    /// should strip the tag before calling this.
    pub fn receive(data: &[u8]) -> Result<VoicePacket, VoiceError> {
        let packet: VoicePacket =
            bincode::deserialize(data).map_err(|e| VoiceError::Transport(format!("{e}")))?;
        let sig_arr: [u8; 64] = packet
            .signature
            .as_slice()
            .try_into()
            .map_err(|_| VoiceError::Transport("voice packet signature length".into()))?;
        let sender_arr: [u8; 32] = packet
            .sender_key
            .as_slice()
            .try_into()
            .map_err(|_| VoiceError::Transport("voice packet sender_key length".into()))?;
        use ed25519_dalek::{Signature, Verifier, VerifyingKey};
        let vk = VerifyingKey::from_bytes(&sender_arr)
            .map_err(|e| VoiceError::Transport(format!("voice packet sender_key invalid: {e}")))?;
        let sig = Signature::from_bytes(&sig_arr);
        vk.verify(&packet.signing_bytes(), &sig)
            .map_err(|e| VoiceError::Transport(format!("voice packet signature: {e}")))?;
        Ok(packet)
    }

    /// Whether the transport has any connected peers.
    pub fn is_connected(&self) -> bool {
        !self.peers.is_empty()
    }

    /// The channel ID this transport is for.
    pub fn channel_id(&self) -> &str {
        &self.channel_id
    }

    /// Build the wire-format packet data from an encoded frame.
    ///
    /// Wave 13 W13.14 — for 1:1 DM calls (`call_key` installed), the
    /// Opus payload is AEAD-encrypted under ChaCha20-Poly1305 with a
    /// deterministic per-packet nonce derived from `(sequence,
    /// timestamp)`. The 16-byte tag is appended; receivers reconstruct
    /// the same nonce and verify+decrypt. For community voice
    /// (`call_key.is_none()`), the audio_data passes through and the
    /// per-channel MEK applied at a higher layer remains the encryption.
    fn build_packet_data(&self, frame: &EncodedFrame) -> Result<Vec<u8>, VoiceError> {
        let signing_key = self
            .signing_key
            .as_ref()
            .ok_or_else(|| VoiceError::Transport("voice signing key not installed".into()))?;

        // W13.14 — encrypt audio_data with ChaCha20-Poly1305 if a
        // call_key is installed.
        let audio_data = if let Some(key) = self.call_key.as_ref() {
            encrypt_audio(key, frame.sequence, frame.timestamp, &frame.data)?
        } else {
            frame.data.clone()
        };

        let mut packet = VoicePacket {
            sender_key: self.sender_key.clone(),
            sequence: frame.sequence,
            timestamp: frame.timestamp,
            audio_data,
            signature: Vec::new(),
        };
        use ed25519_dalek::Signer;
        let sig = signing_key.sign(&packet.signing_bytes());
        packet.signature = sig.to_bytes().to_vec();

        let payload =
            bincode::serialize(&packet).map_err(|e| VoiceError::Transport(format!("{e}")))?;

        // Prepend voice type tag (b'V') so the dispatch loop can distinguish
        // voice packets from chat messages and community broadcasts.
        let mut data = Vec::with_capacity(1 + payload.len());
        data.push(b'V');
        data.extend_from_slice(&payload);
        Ok(data)
    }
}
