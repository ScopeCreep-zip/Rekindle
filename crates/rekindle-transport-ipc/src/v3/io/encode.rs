//! Outbound frame encoder — converts `OutboundFrame` into wire bytes.
//!
//! `FrameEncoder` is the ONLY module that constructs Envelope and Header
//! byte layouts for outbound traffic. No other module serializes wire bytes.
//!
//! Key material is held in a single `parking_lot::RwLock<EncoderState>`.
//! One lock covers epoch + envelope_key + header_key + cipher — no TOCTOU
//! window, no torn reads across rotation. Rayon workers snapshot all fields
//! under one read lock acquisition (~5ns), release the lock, then encrypt
//! with the snapshot. The lock is NEVER held across AEAD seal.
//!
//! Nonce exhaustion calls std::process::abort() — uncatchable by rayon's
//! panic handler. AES-GCM nonce reuse is catastrophic and unrecoverable.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use crate::v3::codec::aead::{self, FrameCipher};
use crate::v3::codec::envelope::{self, EnvelopeInfo};
use crate::v3::codec::header::{self, StreamHeaderInfo};
use crate::v3::context::OutboundFrame;
use crate::v3::wire::constants::WIRE_VERSION;
use crate::v3::wire::envelope::flags as env_flags;
use crate::v3::wire::frame_class::FrameClass;
use crate::v3::wire::lane::Lane;

/// Safety margin: abort 2^20 (~1M) nonces before u64::MAX.
const NONCE_LIMIT: u64 = u64::MAX - (1 << 20);

/// The result of encoding an OutboundFrame.
pub struct EncodedFrame {
    pub session_seq: u64,
    wire_bytes: Vec<u8>,
}

impl EncodedFrame {
    pub fn wire_bytes(&self) -> &[u8] { &self.wire_bytes }
    pub fn into_wire_bytes(self) -> Vec<u8> { self.wire_bytes }
    pub fn wire_len(&self) -> usize { self.wire_bytes.len() }
}

impl AsRef<[u8]> for EncodedFrame {
    fn as_ref(&self) -> &[u8] { &self.wire_bytes }
}

/// Encoded frame plus pre-computed audit chain hashes.
pub struct EncodedWithAudit {
    pub encoded: EncodedFrame,
    pub lane: Lane,
    pub link_input: crate::v3::audit::chain::LinkInput,
}

/// All key material for one epoch. Clone is 2×memcpy(32) + FrameCipher clone
/// (Arc increment on the inner BulkAead).
///
/// Forward secrecy: dropping an EpochKeys instance does NOT guarantee
/// immediate forward secrecy. Rayon workers that cloned this EpochKeys
/// before retirement still hold live copies. Forward secrecy is achieved
/// when ALL clones are dropped — i.e., when the last in-flight rayon
/// worker completes. The 100ms epoch retirement window ensures this.
#[derive(Clone, zeroize::Zeroize)]
pub struct EpochKeys {
    #[zeroize(skip)] // envelope_key is [u8;32] — Zeroize handles it, but we need ZeroizeOnDrop below
    pub envelope_key: [u8; 32],
    pub header_key: [u8; 32],
    #[zeroize(skip)] // FrameCipher wraps Arc<dyn BulkAead> — AEAD impl handles its own key zeroization
    pub cipher: FrameCipher,
}

impl Drop for EpochKeys {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.envelope_key.zeroize();
        self.header_key.zeroize();
    }
}

/// The encoder's full state under a single RwLock.
/// Epoch and keys are ALWAYS read atomically in one lock acquisition.
#[derive(Clone)]
pub struct EncoderState {
    /// 0 or 1. Toggled on each rotation. Stamped into envelope flags bit 4.
    pub epoch: u8,
    /// Keys for the current epoch.
    pub keys: EpochKeys,
}

/// Outbound frame encoder. Single monotonic session_seq counter shared
/// across all lanes. One direction, one key set, one epoch.
pub struct FrameEncoder {
    state: parking_lot::RwLock<EncoderState>,
    session_seq: Arc<AtomicU64>,
}

impl FrameEncoder {
    pub fn new(envelope_key: [u8; 32], header_key: [u8; 32], cipher: FrameCipher) -> Self {
        Self {
            state: parking_lot::RwLock::new(EncoderState {
                epoch: 0,
                keys: EpochKeys { envelope_key, header_key, cipher },
            }),
            session_seq: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn session_seq_counter(&self) -> &Arc<AtomicU64> { &self.session_seq }

    /// Snapshot current epoch + keys atomically. One read lock acquisition.
    /// Rayon workers call this before spawn, capturing the snapshot in the
    /// closure. The lock is released before any AEAD operation.
    pub fn snapshot(&self) -> EncoderState {
        self.state.read().clone()
    }

    /// Convenience accessors for callers that don't need the epoch.
    pub fn envelope_key(&self) -> [u8; 32] { self.state.read().keys.envelope_key }
    pub fn header_key(&self) -> [u8; 32] { self.state.read().keys.header_key }
    pub fn cipher_ref(&self) -> FrameCipher { self.state.read().keys.cipher.clone() }

    /// Atomically install new epoch keys and toggle the epoch counter.
    /// Returns the new epoch number. Called by the read task via
    /// `apply_epoch_signal` — the read task is the sole installer.
    /// One write lock acquisition.
    pub fn install_next_epoch(&self, keys: EpochKeys) -> u8 {
        let mut guard = self.state.write();
        let old_epoch = guard.epoch;
        let old_key_fp = hex::encode(&guard.keys.envelope_key[..8]);
        guard.epoch ^= 1;
        let new_key_fp = hex::encode(&keys.envelope_key[..8]);
        guard.keys = keys;
        tracing::debug!(
            old_epoch,
            new_epoch = guard.epoch,
            old_key_fp = %old_key_fp,
            new_key_fp = %new_key_fp,
            "FrameEncoder::install_next_epoch"
        );
        guard.epoch
    }

    /// Current epoch without cloning keys. For logging/diagnostics only.
    pub fn current_epoch(&self) -> u8 { self.state.read().epoch }

    fn next_session_seq(&self) -> u64 {
        let seq = self.session_seq.fetch_add(1, Ordering::Relaxed);
        if seq >= NONCE_LIMIT {
            let _ = std::io::Write::write_fmt(
                &mut std::io::stderr(),
                format_args!(
                    "FATAL: nonce exhaustion at encoder {:p} session_seq={seq} limit={NONCE_LIMIT} pid={}. \
                     AEAD nonce reuse. Aborting.\n",
                    &self.session_seq as *const _, std::process::id(),
                ),
            );
            std::process::abort();
        }
        seq
    }

    /// Allocate the next session_seq atomically. Called by BulkSender
    /// on the tokio thread to pre-allocate seqs before rayon dispatch.
    pub fn next_session_seq_atomic(&self) -> u64 { self.next_session_seq() }

    /// Encode an OutboundFrame and compute audit chain hashes in one pass.
    pub fn encode_with_audit(&self, frame: &OutboundFrame) -> EncodedWithAudit {
        use crate::v3::audit::chain::LinkInput;
        use crate::v3::wire::constants::{ENVELOPE_LEN, STREAM_HEADER_LEN};

        let lane = frame.lane();
        let encoded = self.encode(frame);
        let wire = encoded.wire_bytes();

        let envelope_hash = *blake3::hash(&wire[..ENVELOPE_LEN]).as_bytes();
        let body = &wire[ENVELOPE_LEN..];
        let header_hash = if lane == Lane::Data && body.len() >= STREAM_HEADER_LEN {
            *blake3::hash(&body[..STREAM_HEADER_LEN]).as_bytes()
        } else {
            [0u8; 32]
        };
        let ciphertext_hash = *blake3::hash(body).as_bytes();

        let link_input = LinkInput {
            session_seq: encoded.session_seq,
            envelope_hash, header_hash, ciphertext_hash,
        };

        EncodedWithAudit { encoded, lane, link_input }
    }

    /// Encode an OutboundFrame into wire bytes.
    pub fn encode(&self, frame: &OutboundFrame) -> EncodedFrame {
        match frame {
            OutboundFrame::Channel { kind, payload } => {
                self.encode_non_data(Lane::Control, FrameClass::Channel as u8, *kind as u8, payload)
            }
            OutboundFrame::Datagram { kind, payload } => {
                self.encode_non_data(Lane::Control, FrameClass::Datagram as u8, *kind as u8, payload)
            }
            OutboundFrame::Audit { kind, payload } => {
                self.encode_non_data(Lane::Audit, FrameClass::Audit as u8, *kind as u8, payload)
            }
            OutboundFrame::Handoff { kind, payload } => {
                self.encode_non_data(Lane::Handoff, FrameClass::Handoff as u8, *kind as u8, payload)
            }
            OutboundFrame::Data { stream_id, kind, chunk_index, payload } => {
                self.encode_data(*stream_id, *kind, *chunk_index, payload)
            }
        }
    }

    fn encode_non_data(&self, lane: Lane, class: u8, kind: u8, payload: &[u8]) -> EncodedFrame {
        let seq = self.next_session_seq();

        // Snapshot epoch + keys atomically. Lock released before AEAD.
        let snap = self.snapshot();

        let mut plaintext = Vec::with_capacity(2 + payload.len());
        plaintext.push(class);
        plaintext.push(kind);
        plaintext.extend_from_slice(payload);

        let body_len = u32::try_from(aead::non_data_body_len(plaintext.len())).expect("body exceeds u32");

        let epoch_flag = if snap.epoch & 1 == 1 { env_flags::KEY_EPOCH } else { 0 };
        let env_info = EnvelopeInfo {
            wire_version: WIRE_VERSION, lane, flags: epoch_flag, body_len, session_seq: seq,
        };
        let envelope_bytes = envelope::build_envelope(&env_info, &snap.keys.envelope_key);
        let ciphertext = snap.keys.cipher.seal(seq, &envelope_bytes, None, &plaintext);

        let mut wire = Vec::with_capacity(32 + ciphertext.len());
        wire.extend_from_slice(&envelope_bytes);
        wire.extend_from_slice(&ciphertext);
        EncodedFrame { session_seq: seq, wire_bytes: wire }
    }

    fn encode_data(&self, stream_id: u8, kind: crate::v3::wire::frame_kind::StreamKind, chunk_index: u32, payload: &[u8]) -> EncodedFrame {
        let seq = self.next_session_seq();

        // Snapshot epoch + keys atomically. Lock released before AEAD.
        let snap = self.snapshot();

        let header_info = StreamHeaderInfo {
            frame_class: FrameClass::Stream,
            frame_kind: kind,
            stream_id,
            header_flags: 0,
            chunk_index,
            nonce: seq,
        };
        let header_bytes = header::build_header(&header_info, &snap.keys.header_key);

        let body_len = u32::try_from(aead::data_body_len(payload.len())).expect("body exceeds u32");
        let epoch_flag = if snap.epoch & 1 == 1 { env_flags::KEY_EPOCH } else { 0 };
        let env_info = EnvelopeInfo {
            wire_version: WIRE_VERSION, lane: Lane::Data, flags: epoch_flag, body_len, session_seq: seq,
        };
        let envelope_bytes = envelope::build_envelope(&env_info, &snap.keys.envelope_key);
        let ciphertext = snap.keys.cipher.seal(seq, &envelope_bytes, Some(&header_bytes), payload);

        let mut wire = Vec::with_capacity(32 + 32 + ciphertext.len());
        wire.extend_from_slice(&envelope_bytes);
        wire.extend_from_slice(&header_bytes);
        wire.extend_from_slice(&ciphertext);
        EncodedFrame { session_seq: seq, wire_bytes: wire }
    }
}
