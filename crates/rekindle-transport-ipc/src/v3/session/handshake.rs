//! Noise IK handshake with encrypted CHANNEL_HELLO exchange,
//! capability/clearance reconciliation, AEAD algorithm negotiation,
//! and construction of ready-to-use FrameEncoder + FrameDecoder.
//!
//! The handshake is the SINGLE SOURCE OF TRUTH for:
//! - Key direction assignment (dialler=d2l outbound, listener=l2d outbound)
//! - AEAD algorithm selection (runtime probe: fastest of AEGIS-128X2/128L, FIPS→AES-256-GCM)
//! - FrameCipher construction with direction_id baked in
//! - FrameEncoder/FrameDecoder construction with correct keys
//!
//! Downstream consumers receive `HandshakeResult` containing ready-to-use
//! encoder and decoder. They never touch a key, direction_id, or algorithm
//! enum. When the handshake changes, zero downstream files change.
//!
//! The handshake has three phases:
//! 1. Noise IK msg1/msg2 exchange (key agreement + mutual authentication)
//! 2. Encrypted CHANNEL_HELLO/HELLO_ACK exchange (via Noise stateless transport)
//! 3. Capability + clearance reconciliation, AEAD algorithm selection, key derivation
//!
//! After phase 3, the Noise transport state is dropped. All subsequent traffic
//! uses HKDF-derived keys through the v3 Envelope+AEAD pipeline.

use std::sync::Arc;
use std::sync::atomic::AtomicU64;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use rekindle_aead::BulkAead as _;
use crate::v3::codec::aead::FrameCipher;
use crate::v3::codec::channel::{hello, hello_ack};
use crate::v3::crypto::keys::{derive_all_keys, DerivedKeys};
use crate::v3::crypto::noise::{
    build_initiator, build_responder, generate_keypair, NoiseError,
};
use crate::v3::context::SessionRole;
use crate::v3::io::decode::FrameDecoder;
use crate::v3::io::encode::{EpochKeys, FrameEncoder};
use crate::v3::wire::capability::CapabilityBits;
use crate::v3::wire::clearance::Clearance;
use crate::v3::wire::constants::{DIRECTION_ID_D2L, DIRECTION_ID_L2D};

/// Configuration for one side of a handshake.
///
/// Construct via [`HandshakeConfig::new`] or [`HandshakeConfig::default`],
/// then use the builder methods to customize. New fields added in future
/// versions will have defaults — existing code is not broken.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct HandshakeConfig {
    pub capabilities: CapabilityBits,
    pub clearance: Clearance,
    /// Explicit AEAD algorithm preference. When `Some`, the negotiation
    /// uses this algorithm instead of runtime-probing. Takes precedence
    /// over the runtime probe but NOT over FIPS mode. `None` = runtime probe.
    pub preferred_aead: Option<u8>,
}

impl HandshakeConfig {
    /// Create with explicit capabilities and clearance. All other fields
    /// are set to their defaults.
    pub fn new(capabilities: CapabilityBits, clearance: Clearance) -> Self {
        Self {
            capabilities,
            clearance,
            preferred_aead: None,
        }
    }

    /// Set the preferred AEAD algorithm.
    pub fn with_preferred_aead(mut self, aead: u8) -> Self {
        self.preferred_aead = Some(aead);
        self
    }
}

impl Default for HandshakeConfig {
    fn default() -> Self {
        Self {
            capabilities: CapabilityBits::MANDATORY_V1,
            clearance: Clearance::Internal,
            preferred_aead: None,
        }
    }
}

/// Successful handshake result.
///
/// Contains everything needed to run a session — including ready-to-use
/// `FrameEncoder` and `FrameDecoder` with correct direction, keys, and
/// algorithm baked in. Consumers never construct ciphers or select directions.
///
/// NOT Clone — encoder/decoder hold AtomicU64 counters and Arc<dyn BulkAead>.
/// Test helpers that need the result's fields should read them before moving
/// the encoder/decoder into tasks.
pub struct HandshakeResult {
    pub session_id: uuid::Uuid,
    pub local_peer_id: [u8; 32],
    pub remote_peer_id: [u8; 32],
    pub active_capabilities: CapabilityBits,
    pub agreed_clearance: Clearance,
    pub agreed_aead: u8,
    pub handshake_hash: [u8; 32],
    /// All 9 HKDF-derived keys. Retained for audit chain (audit keys),
    /// handoff MAC (handoff key), and key rotation (re-derivation input).
    /// NOT used for cipher construction — that's done by encoder/decoder.
    pub keys: DerivedKeys,
    /// Outbound encoder with correct direction + algorithm.
    /// Server: l2d keys, DIRECTION_ID_L2D. Client: d2l keys, DIRECTION_ID_D2L.
    pub encoder: FrameEncoder,
    /// Inbound decoder with correct direction + algorithm.
    /// Server: d2l keys, DIRECTION_ID_D2L. Client: l2d keys, DIRECTION_ID_L2D.
    pub decoder: FrameDecoder,
}

// HandshakeResult needs Debug for tracing/error messages but encoder/decoder
// don't implement Debug, so we implement it manually.
impl std::fmt::Debug for HandshakeResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HandshakeResult")
            .field("session_id", &self.session_id)
            .field("active_capabilities", &self.active_capabilities)
            .field("agreed_clearance", &self.agreed_clearance)
            .field("agreed_aead", &self.agreed_aead)
            .finish_non_exhaustive()
    }
}

/// Named handshake failures.
#[derive(Debug)]
pub enum HandshakeError {
    Timeout,
    NoiseFailed { detail: String },
    CapabilityMismatch { missing: CapabilityBits },
    PeerUnregistered,
    PeerUidDisallowed,
    PeerClearanceMismatch,
    TranscriptMismatch,
    WireVersionUnsupported,
    InvalidCredentials,
    SubstrateError { detail: String },
    CipherInitFailed { detail: String },
}

impl From<NoiseError> for HandshakeError {
    fn from(e: NoiseError) -> Self {
        Self::NoiseFailed { detail: format!("{e:?}") }
    }
}

impl From<std::io::Error> for HandshakeError {
    fn from(e: std::io::Error) -> Self {
        Self::SubstrateError { detail: format!("{e}") }
    }
}

// ── AEAD algorithm negotiation ──────────────────────────────────

/// AEAD algorithm codes on the wire (HELLO_ACK.agreed_aead field).
pub const AEAD_CODE_AES256GCM: u8 = 0x01;
pub const AEAD_CODE_AEGIS128L: u8 = 0x03;
pub const AEAD_CODE_AEGIS128X2: u8 = 0x04;

/// Select the highest-performance AEAD both sides support.
/// FIPS mode forces AES-256-GCM regardless of AEGIS availability.
///
/// When AEGIS is available, probes both AEGIS-128L and AEGIS-128X2
/// at runtime and selects the faster one for this CPU. X2 is the
/// 2-way parallel variant that benefits from VAES (Zen 3+, Ice Lake+).
/// On CPUs with a single AES execution port (Coffee Lake, Zen 2),
/// 128L is faster because X2's 2x round count serializes on the port.
/// The probe runs once per process and is cached.
fn negotiate_aead(active: CapabilityBits, preferred: Option<u8>) -> u8 {
    // 1. FIPS/regulatory override — always wins, no choice.
    if active.contains(CapabilityBits::FIPS_MODE) {
        return AEAD_CODE_AES256GCM;
    }
    // 2. Explicit user/system preference — honored if the capability exists.
    if let Some(pref) = preferred {
        match pref {
            AEAD_CODE_AEGIS128L | AEAD_CODE_AEGIS128X2
                if active.contains(CapabilityBits::AEAD_AEGIS128L) => return pref,
            AEAD_CODE_AES256GCM => return AEAD_CODE_AES256GCM,
            _ => {} // unknown preference, fall through to probe
        }
    }
    // 3. Runtime probe — pick the fastest available.
    if active.contains(CapabilityBits::AEAD_AEGIS128L) {
        return probe_best_aegis();
    }
    AEAD_CODE_AES256GCM
}

/// Probe AEGIS-128L vs AEGIS-128X2 throughput and return the faster one.
/// Cached in a OnceLock — runs once per process.
fn probe_best_aegis() -> u8 {
    use std::sync::OnceLock;
    static BEST: OnceLock<u8> = OnceLock::new();
    *BEST.get_or_init(|| {
        let key_128l = rekindle_aead::aegis128l::Aegis128LKey::new(&[0x42; 16]);
        let key_x2 = rekindle_aead::aegis128x2::Aegis128X2Key::new(&[0x42; 16]);

        // Probe at 64 KiB — the production chunk AEAD size.
        let probe_data = vec![0xAB; 65536];
        let nonce_l = key_128l.build_nonce(0);
        let nonce_x2 = key_x2.build_nonce(0);
        let aad = [0u8; 64];

        // Warmup: 4 rounds each to stabilize CPU frequency + fill caches.
        for _ in 0..4 {
            let mut ct = vec![0u8; probe_data.len()];
            let mut tag = [0u8; 16];
            let _ = key_128l.seal_detached(&nonce_l, &aad, &probe_data, &mut ct, &mut tag);
            let _ = key_x2.seal_detached(&nonce_x2, &aad, &probe_data, &mut ct, &mut tag);
        }

        // Measure: 16 rounds each, take total time.
        let rounds = 16;
        let mut ct = vec![0u8; probe_data.len()];
        let mut tag = [0u8; 16];

        let start_l = std::time::Instant::now();
        for _ in 0..rounds {
            let _ = key_128l.seal_detached(&nonce_l, &aad, &probe_data, &mut ct, &mut tag);
        }
        let elapsed_l = start_l.elapsed();

        let start_x2 = std::time::Instant::now();
        for _ in 0..rounds {
            let _ = key_x2.seal_detached(&nonce_x2, &aad, &probe_data, &mut ct, &mut tag);
        }
        let elapsed_x2 = start_x2.elapsed();

        if elapsed_x2 < elapsed_l {
            tracing::info!(
                aegis128l_ns = elapsed_l.as_nanos(),
                aegis128x2_ns = elapsed_x2.as_nanos(),
                "AEAD probe: AEGIS-128X2 faster — selecting X2"
            );
            AEAD_CODE_AEGIS128X2
        } else {
            tracing::info!(
                aegis128l_ns = elapsed_l.as_nanos(),
                aegis128x2_ns = elapsed_x2.as_nanos(),
                "AEAD probe: AEGIS-128L faster — selecting 128L"
            );
            AEAD_CODE_AEGIS128L
        }
    })
}

/// Construct a FrameCipher from the negotiated AEAD code and key bytes.
/// Public for BulkReceiver cipher construction in server.rs and client/mod.rs
/// which must use the same algorithm the handshake negotiated.
pub fn build_bulk_cipher(
    aead_code: u8,
    key_bytes: &[u8; 32],
    direction_id: [u8; 4],
) -> Result<FrameCipher, HandshakeError> {
    match aead_code {
        AEAD_CODE_AEGIS128X2 => Ok(FrameCipher::aegis128x2(key_bytes, direction_id)),
        AEAD_CODE_AEGIS128L => {
            let key_16: [u8; 16] = key_bytes[..16].try_into().expect("32 >= 16");
            let key = rekindle_aead::aegis128l::Aegis128LKey::new(&key_16);
            Ok(FrameCipher::new(Arc::new(key), direction_id))
        }
        AEAD_CODE_AES256GCM | _ => {
            FrameCipher::aes256gcm(key_bytes, direction_id)
                .map_err(|e| HandshakeError::CipherInitFailed { detail: format!("{e:?}") })
        }
    }
}

/// Build EpochKeys for one direction (encoder or decoder) from DerivedKeys.
///
/// Used by key rotation handlers to construct epoch-aware key material
/// without importing wire constants. The direction mapping (role → d2l/l2d)
/// is the SSOT defined here, not in handler code.
///
/// `for_encoder`: true = our outbound direction, false = their outbound (our inbound).
pub fn build_epoch_keys(
    keys: &DerivedKeys,
    role: SessionRole,
    aead_code: u8,
    for_encoder: bool,
) -> Result<EpochKeys, HandshakeError> {
    let (env_key, hdr_key, stream_key, dir) = match (role, for_encoder) {
        (SessionRole::Dialler, true)  => (keys.envelope_d2l, keys.header_d2l, &keys.stream_d2l, DIRECTION_ID_D2L),
        (SessionRole::Listener, true) => (keys.envelope_l2d, keys.header_l2d, &keys.stream_l2d, DIRECTION_ID_L2D),
        (SessionRole::Dialler, false) => (keys.envelope_l2d, keys.header_l2d, &keys.stream_l2d, DIRECTION_ID_L2D),
        (SessionRole::Listener, false)=> (keys.envelope_d2l, keys.header_d2l, &keys.stream_d2l, DIRECTION_ID_D2L),
    };

    let cipher = build_bulk_cipher(aead_code, stream_key, dir)?;
    tracing::debug!(
        role = ?role,
        for_encoder,
        envelope_key_fp = %hex::encode(&env_key[..8]),
        header_key_fp = %hex::encode(&hdr_key[..8]),
        direction = %hex::encode(dir),
        "build_epoch_keys: constructed"
    );
    Ok(EpochKeys { envelope_key: env_key, header_key: hdr_key, cipher })
}

/// Build encoder + decoder for a specific role after key derivation.
///
/// `pub(crate)` so test helpers can construct real encoder/decoder from
/// real HKDF-derived keys without running the full Noise exchange.
pub fn build_encoder_decoder(
    keys: &DerivedKeys,
    aead_code: u8,
    is_dialler: bool,
) -> Result<(FrameEncoder, FrameDecoder), HandshakeError> {
    // Dialler outbound = d2l, inbound = l2d
    // Listener outbound = l2d, inbound = d2l
    let (send_stream_key, send_dir, send_env_key, send_hdr_key,
         recv_stream_key, recv_dir, recv_env_key, recv_hdr_key) = if is_dialler {
        (&keys.stream_d2l, DIRECTION_ID_D2L, keys.envelope_d2l, keys.header_d2l,
         &keys.stream_l2d, DIRECTION_ID_L2D, keys.envelope_l2d, keys.header_l2d)
    } else {
        (&keys.stream_l2d, DIRECTION_ID_L2D, keys.envelope_l2d, keys.header_l2d,
         &keys.stream_d2l, DIRECTION_ID_D2L, keys.envelope_d2l, keys.header_d2l)
    };

    let send_cipher = build_bulk_cipher(aead_code, send_stream_key, send_dir)?;
    let recv_cipher = build_bulk_cipher(aead_code, recv_stream_key, recv_dir)?;

    let encoder = FrameEncoder::new(send_env_key, send_hdr_key, send_cipher);
    let decoder = FrameDecoder::new(recv_env_key, recv_hdr_key, recv_cipher);

    Ok((encoder, decoder))
}

// ── Wire-level Noise message framing ──────────────────────────────

async fn write_noise_msg<W: AsyncWrite + Unpin>(
    writer: &mut W,
    data: &[u8],
) -> Result<(), HandshakeError> {
    let len = u32::try_from(data.len())
        .expect("noise message exceeds u32 length")
        .to_le_bytes();
    writer.write_all(&len).await?;
    writer.write_all(data).await?;
    writer.flush().await?;
    Ok(())
}

async fn read_noise_msg<R: AsyncRead + Unpin>(
    reader: &mut R,
) -> Result<Vec<u8>, HandshakeError> {
    let mut len_buf = [0u8; 4];
    reader.read_exact(&mut len_buf).await?;
    let len = u32::from_le_bytes(len_buf) as usize;
    if len > 65535 {
        return Err(HandshakeError::NoiseFailed {
            detail: format!("noise msg too large: {len}"),
        });
    }
    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf).await?;
    Ok(buf)
}

/// Encrypt a payload through the Noise stateless transport and send it.
async fn write_encrypted<W: AsyncWrite + Unpin>(
    writer: &mut W,
    transport: &snow::StatelessTransportState,
    nonce: &AtomicU64,
    plaintext: &[u8],
) -> Result<(), HandshakeError> {
    let n = nonce.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut buf = vec![0u8; plaintext.len() + 16];
    let len = transport.write_message(n, plaintext, &mut buf)
        .map_err(|e| HandshakeError::NoiseFailed { detail: format!("encrypt: {e}") })?;
    write_noise_msg(writer, &buf[..len]).await
}

/// Read and decrypt a payload through the Noise stateless transport.
async fn read_encrypted<R: AsyncRead + Unpin>(
    reader: &mut R,
    transport: &snow::StatelessTransportState,
    nonce: &AtomicU64,
) -> Result<Vec<u8>, HandshakeError> {
    let ciphertext = read_noise_msg(reader).await?;
    let n = nonce.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut buf = vec![0u8; ciphertext.len()];
    let len = transport.read_message(n, &ciphertext, &mut buf)
        .map_err(|e| HandshakeError::NoiseFailed { detail: format!("decrypt: {e}") })?;
    buf.truncate(len);
    Ok(buf)
}

/// Wall-clock nanoseconds since Unix epoch for HELLO epoch_ns fields.
fn wall_ns() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
}

// ── Dialler-side handshake ────────────────────────────────────────

/// Run the dialler (initiator) side of the v3 handshake.
///
/// Returns a `HandshakeResult` with ready-to-use `encoder` (d2l direction)
/// and `decoder` (l2d direction). The caller splits the stream AFTER this
/// returns and passes encoder/decoder to the write/read tasks.
pub async fn dialler_handshake<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    local_keypair: &snow::Keypair,
    remote_public_key: &[u8],
    config: HandshakeConfig,
    prologue: &[u8],
    timeout: std::time::Duration,
) -> Result<HandshakeResult, HandshakeError> {
    tokio::time::timeout(timeout, async {
        let mut hs = build_initiator(&local_keypair.private, remote_public_key, prologue)?;

        // Noise msg1: dialler → listener
        let mut msg1_buf = vec![0u8; 65535];
        let msg1_len = hs.write_message(&[], &mut msg1_buf)
            .map_err(|e| HandshakeError::NoiseFailed { detail: format!("msg1 write: {e}") })?;
        write_noise_msg(stream, &msg1_buf[..msg1_len]).await?;

        // Noise msg2: listener → dialler
        let msg2 = read_noise_msg(stream).await?;
        let mut payload_buf = vec![0u8; 65535];
        hs.read_message(&msg2, &mut payload_buf)
            .map_err(|e| HandshakeError::NoiseFailed { detail: format!("msg2 read: {e}") })?;

        // Extract handshake hash before transition
        let hash: [u8; 32] = hs.get_handshake_hash()
            .try_into()
            .map_err(|_| HandshakeError::NoiseFailed { detail: "hash not 32 bytes".into() })?;

        // Transition to stateless transport for encrypted HELLO exchange
        let transport = Arc::new(
            hs.into_stateless_transport_mode()
                .map_err(|e| HandshakeError::NoiseFailed { detail: format!("transport mode: {e}") })?
        );
        let send_nonce = AtomicU64::new(0);
        let recv_nonce = AtomicU64::new(0);

        // CHANNEL_HELLO: dialler → listener (encrypted through Noise transport)
        let local_peer_id: [u8; 32] = *blake3::hash(&local_keypair.public).as_bytes();
        let session_id = uuid::Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext));

        let hello_payload = hello::HelloPayload {
            session_id_proposal: session_id,
            dialler_peer_id: local_peer_id,
            capabilities: config.capabilities,
            proposed_clearance: config.clearance,
            dialler_epoch_ns: wall_ns(),
            dialler_handshake_hash: hash,
        };
        let hello_bytes = hello::encode(&hello_payload);
        write_encrypted(stream, &transport, &send_nonce, &hello_bytes).await?;

        // CHANNEL_HELLO_ACK: listener → dialler (encrypted through Noise transport)
        let ack_bytes = read_encrypted(stream, &transport, &recv_nonce).await?;
        let ack = hello_ack::decode(&ack_bytes)
            .map_err(|e| HandshakeError::NoiseFailed { detail: format!("hello_ack decode: {e:?}") })?;

        // Verify transcript hashes match
        if ack.listener_handshake_hash != hash {
            return Err(HandshakeError::TranscriptMismatch);
        }

        // Capability reconciliation
        let active = config.capabilities & ack.capabilities;
        let mandatory = CapabilityBits::MANDATORY_V1;
        if !active.contains(mandatory) {
            let missing = mandatory & !active;
            return Err(HandshakeError::CapabilityMismatch { missing });
        }

        // Noise transport dropped — HKDF keys take over
        drop(transport);

        // Derive all 9 keys from the handshake hash
        let keys = derive_all_keys(&hash);

        // The listener selected the AEAD algorithm in HELLO_ACK
        let agreed_aead = ack.agreed_aead;

        // Build encoder (d2l) and decoder (l2d) with correct direction + algorithm
        let (encoder, decoder) = build_encoder_decoder(&keys, agreed_aead, true)?;

        Ok(HandshakeResult {
            session_id: ack.session_id,
            local_peer_id,
            remote_peer_id: ack.listener_peer_id,
            active_capabilities: active,
            agreed_clearance: ack.agreed_clearance,
            agreed_aead,
            handshake_hash: hash,
            keys,
            encoder,
            decoder,
        })
    })
    .await
    .map_err(|_| HandshakeError::Timeout)?
}

// ── Listener-side handshake ───────────────────────────────────────

/// Run the listener (responder) side of the v3 handshake.
///
/// Returns a `HandshakeResult` with ready-to-use `encoder` (l2d direction)
/// and `decoder` (d2l direction).
pub async fn listener_handshake<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    local_keypair: &snow::Keypair,
    config: HandshakeConfig,
    prologue: &[u8],
    timeout: std::time::Duration,
) -> Result<HandshakeResult, HandshakeError> {
    tokio::time::timeout(timeout, async {
        let mut hs = build_responder(&local_keypair.private, prologue)?;

        // Noise msg1: dialler → listener
        let msg1 = read_noise_msg(stream).await?;
        let mut payload_buf = vec![0u8; 65535];
        hs.read_message(&msg1, &mut payload_buf)
            .map_err(|e| HandshakeError::NoiseFailed { detail: format!("msg1 read: {e}") })?;

        // Noise msg2: listener → dialler
        let mut msg2_buf = vec![0u8; 65535];
        let msg2_len = hs.write_message(&[], &mut msg2_buf)
            .map_err(|e| HandshakeError::NoiseFailed { detail: format!("msg2 write: {e}") })?;
        write_noise_msg(stream, &msg2_buf[..msg2_len]).await?;

        // Extract handshake hash before transition
        let hash: [u8; 32] = hs.get_handshake_hash()
            .try_into()
            .map_err(|_| HandshakeError::NoiseFailed { detail: "hash not 32 bytes".into() })?;

        // Transition to stateless transport for encrypted HELLO exchange
        let transport = Arc::new(
            hs.into_stateless_transport_mode()
                .map_err(|e| HandshakeError::NoiseFailed { detail: format!("transport mode: {e}") })?
        );
        let send_nonce = AtomicU64::new(0);
        let recv_nonce = AtomicU64::new(0);

        // CHANNEL_HELLO: dialler → listener (encrypted through Noise transport)
        let hello_bytes = read_encrypted(stream, &transport, &recv_nonce).await?;
        let hello = hello::decode(&hello_bytes)
            .map_err(|e| HandshakeError::NoiseFailed { detail: format!("hello decode: {e:?}") })?;

        // Verify transcript hashes match
        if hello.dialler_handshake_hash != hash {
            return Err(HandshakeError::TranscriptMismatch);
        }

        // Capability reconciliation
        let active = config.capabilities & hello.capabilities;
        let mandatory = CapabilityBits::MANDATORY_V1;
        if !active.contains(mandatory) {
            let missing = mandatory & !active;
            return Err(HandshakeError::CapabilityMismatch { missing });
        }

        // Clearance reconciliation
        let agreed_clearance = std::cmp::min(config.clearance, hello.proposed_clearance);

        // AEAD algorithm selection — listener decides, dialler accepts
        let agreed_aead = negotiate_aead(active, config.preferred_aead);

        // CHANNEL_HELLO_ACK: listener → dialler (encrypted through Noise transport)
        let local_peer_id: [u8; 32] = *blake3::hash(&local_keypair.public).as_bytes();

        let ack_payload = hello_ack::HelloAckPayload {
            session_id: hello.session_id_proposal,
            listener_peer_id: local_peer_id,
            capabilities: config.capabilities,
            agreed_clearance,
            agreed_aead,
            listener_epoch_ns: wall_ns(),
            listener_handshake_hash: hash,
        };
        let ack_bytes = hello_ack::encode(&ack_payload);
        write_encrypted(stream, &transport, &send_nonce, &ack_bytes).await?;

        // Noise transport dropped — HKDF keys take over
        drop(transport);

        // Derive all 9 keys from the handshake hash
        let keys = derive_all_keys(&hash);

        // Build encoder (l2d) and decoder (d2l) with correct direction + algorithm
        let (encoder, decoder) = build_encoder_decoder(&keys, agreed_aead, false)?;

        Ok(HandshakeResult {
            session_id: hello.session_id_proposal,
            local_peer_id,
            remote_peer_id: hello.dialler_peer_id,
            active_capabilities: active,
            agreed_clearance,
            agreed_aead,
            handshake_hash: hash,
            keys,
            encoder,
            decoder,
        })
    })
    .await
    .map_err(|_| HandshakeError::Timeout)?
}

// ── Test helpers ──────────────────────────────────────────────────

/// Run a full handshake between two in-process peers over `tokio::io::duplex`.
///
/// Returns both sides' HandshakeResults. Both use the same AEAD algorithm
/// (negotiated by the listener) and matching directional keys.
pub async fn handshake_pair(
    dialler_config: HandshakeConfig,
    listener_config: HandshakeConfig,
) -> Result<(HandshakeResult, HandshakeResult), HandshakeError> {
    let dialler_keypair = generate_keypair()?;
    let listener_keypair = generate_keypair()?;
    let prologue = b"RTI-IPC-v1:100:1000:200:1000";
    let timeout = std::time::Duration::from_secs(5);

    let (dialler_stream, listener_stream) = tokio::io::duplex(65536);
    let (mut dialler_read, mut dialler_write) = tokio::io::split(dialler_stream);
    let (mut listener_read, mut listener_write) = tokio::io::split(listener_stream);

    let listener_pub = listener_keypair.public.clone();
    let dialler_handle = tokio::spawn(async move {
        let mut stream = tokio::io::join(&mut dialler_read, &mut dialler_write);
        dialler_handshake(
            &mut stream,
            &dialler_keypair,
            &listener_pub,
            dialler_config,
            prologue,
            timeout,
        ).await
    });

    let listener_handle = tokio::spawn(async move {
        let mut stream = tokio::io::join(&mut listener_read, &mut listener_write);
        listener_handshake(
            &mut stream,
            &listener_keypair,
            listener_config,
            prologue,
            timeout,
        ).await
    });

    let (dialler_result, listener_result) = tokio::try_join!(
        async { dialler_handle.await.map_err(|e| HandshakeError::SubstrateError { detail: format!("{e}") })? },
        async { listener_handle.await.map_err(|e| HandshakeError::SubstrateError { detail: format!("{e}") })? },
    )?;

    Ok((dialler_result, listener_result))
}

/// Handshake where the dialler uses the wrong listener public key.
pub async fn handshake_with_wrong_key(
    dialler_config: HandshakeConfig,
    listener_config: HandshakeConfig,
) -> Result<(HandshakeResult, HandshakeResult), HandshakeError> {
    let dialler_keypair = generate_keypair()?;
    let listener_keypair = generate_keypair()?;
    let wrong_keypair = generate_keypair()?;
    let prologue = b"RTI-IPC-v1:100:1000:200:1000";
    let timeout = std::time::Duration::from_secs(5);

    let (dialler_stream, listener_stream) = tokio::io::duplex(65536);
    let (mut dialler_read, mut dialler_write) = tokio::io::split(dialler_stream);
    let (mut listener_read, mut listener_write) = tokio::io::split(listener_stream);

    let wrong_pub = wrong_keypair.public.clone();
    let dialler_handle = tokio::spawn(async move {
        let mut stream = tokio::io::join(&mut dialler_read, &mut dialler_write);
        dialler_handshake(
            &mut stream,
            &dialler_keypair,
            &wrong_pub,
            dialler_config,
            prologue,
            timeout,
        ).await
    });

    let listener_handle = tokio::spawn(async move {
        let mut stream = tokio::io::join(&mut listener_read, &mut listener_write);
        listener_handshake(
            &mut stream,
            &listener_keypair,
            listener_config,
            prologue,
            timeout,
        ).await
    });

    let (d, l) = tokio::join!(dialler_handle, listener_handle);
    let d = d.map_err(|e| HandshakeError::SubstrateError { detail: format!("{e}") })?;
    let l = l.map_err(|e| HandshakeError::SubstrateError { detail: format!("{e}") })?;

    match (d, l) {
        (Err(e), _) | (_, Err(e)) => Err(e),
        (Ok(_), Ok(_)) => Err(HandshakeError::NoiseFailed {
            detail: "wrong key accepted — Noise IK should have rejected".into(),
        }),
    }
}

/// Handshake with a real timeout. The listener side is dropped immediately
/// so the dialler's read hangs until `tokio::time::timeout` fires.
pub async fn handshake_with_timeout(
    config: HandshakeConfig,
    timeout: std::time::Duration,
) -> Result<(HandshakeResult, HandshakeResult), HandshakeError> {
    let dialler_keypair = generate_keypair()?;
    let listener_keypair = generate_keypair()?;
    let prologue = b"RTI-IPC-v1:100:1000:200:1000";

    let (dialler_stream, _dropped_listener) = tokio::io::duplex(65536);
    let (mut dr, mut dw) = tokio::io::split(dialler_stream);
    let mut stream = tokio::io::join(&mut dr, &mut dw);

    let result = dialler_handshake(
        &mut stream,
        &dialler_keypair,
        &listener_keypair.public,
        config,
        prologue,
        timeout,
    ).await;

    match result {
        Err(e) => Err(e),
        Ok(_) => Err(HandshakeError::NoiseFailed {
            detail: "handshake succeeded despite dropped listener — should not happen".into(),
        }),
    }
}
