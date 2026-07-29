//! Bulk receive pipeline — rayon-parallel decryption with reassembly.
//!
//! The read task dispatches large Data lane frames to `BulkReceiver` for
//! parallel AEAD decryption on the rayon pool. Rayon workers decrypt into
//! pre-allocated RecvBuf from the recv buffer freelist — zero allocations
//! on rayon worker threads. Results are pushed into a DispatchQueue for
//! the control loop to consume via TokioWake notification.
//!
//! Inbound audit links are carried on DecryptedChunk.link_input and
//! forwarded to audit_merge by the Data lane — NOT pushed to a separate
//! audit queue. One path, one chain, zero duplication.
//!
//! Epoch-aware: after key rotation, the BulkReceiver's key slots are
//! updated via `install_epoch_keys` (called by the read task's
//! `apply_epoch_signal` helper). The rayon closure captures the correct
//! epoch's keys at dispatch time — same snapshot pattern as the encoder.

use std::sync::Arc;

use rayon::ThreadPool;

use rekindle_transport_buff::adapters::tokio::TokioWake;
use rekindle_transport_buff::DispatchQueue;

use crate::v4::audit::chain::LinkInput;
use crate::v4::codec::aead::FrameCipher;
use crate::v4::io::encode::EpochKeys;
use crate::v4::io::lane_channels::{PlaintextBuf, RecvBufPool, WireBuf, WireBufPool};
use crate::v4::wire::constants::{AEAD_TAG_LEN, ENVELOPE_LEN, STREAM_HEADER_LEN};
use crate::v4::wire::frame_kind::StreamKind;

/// A decrypted chunk ready for reassembly, OR a fatal error from a rayon worker.
pub enum BulkDecryptResult {
    Chunk(DecryptedChunk),
    Fatal(RecvDispatchError),
}

/// A decrypted chunk ready for reassembly.
pub struct DecryptedChunk {
    pub stream_id: u8,
    pub chunk_index: u32,
    pub plaintext: PlaintextBuf,
    pub kind: StreamKind,
    pub header_flags: u8,
    pub link_input: LinkInput,
    pub chunk_digest: [u8; 32],
}

/// Dispatch errors from the bulk receive pipeline.
#[derive(Debug)]
pub enum RecvDispatchError {
    FrameTooShort { len: usize },
    EnvelopeMacFailed,
    HeaderMacFailed,
    AeadFailed,
    WorkerPanic,
}

impl std::fmt::Display for RecvDispatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::FrameTooShort { len } => write!(f, "bulk frame too short: {len} bytes"),
            Self::EnvelopeMacFailed => write!(f, "bulk frame EMAC verification failed"),
            Self::HeaderMacFailed => write!(f, "bulk frame HeaderMAC verification failed"),
            Self::AeadFailed => write!(f, "bulk frame AEAD verification failed"),
            Self::WorkerPanic => write!(f, "bulk decrypt rayon worker panicked"),
        }
    }
}

impl std::error::Error for RecvDispatchError {}

pub struct BulkReceiver {
    decrypt_pool: Arc<ThreadPool>,
    result_queue: Arc<DispatchQueue<BulkDecryptResult, TokioWake>>,
    key_slots: [Option<EpochKeys>; 2],
    current_epoch: u8,
    recv_pool: RecvBufPool,
    wire_pool: WireBufPool,
}

impl BulkReceiver {
    pub fn new(
        envelope_key: [u8; 32],
        header_key: [u8; 32],
        cipher: Arc<FrameCipher>,
        decrypt_pool: Arc<ThreadPool>,
        result_queue: Arc<DispatchQueue<BulkDecryptResult, TokioWake>>,
        recv_pool: RecvBufPool,
        wire_pool: WireBufPool,
    ) -> Self {
        let initial_keys = EpochKeys {
            envelope_key,
            header_key,
            cipher: (*cipher).clone(),
        };
        let mut key_slots: [Option<EpochKeys>; 2] = [None, None];
        key_slots[0] = Some(initial_keys);

        Self {
            decrypt_pool, result_queue,
            key_slots,
            current_epoch: 0,
            recv_pool, wire_pool,
        }
    }

    pub fn current_epoch(&self) -> u8 { self.current_epoch }

    pub fn wake(&self) -> &TokioWake {
        self.result_queue.wake_sink()
    }

    pub fn install_epoch_keys(&mut self, epoch: u8, keys: EpochKeys) {
        let slot = (epoch & 1) as usize;
        tracing::info!(
            epoch, slot,
            previous_occupied = self.key_slots[slot].is_some(),
            "BulkReceiver: installing epoch keys"
        );
        self.key_slots[slot] = Some(keys);
        self.current_epoch = epoch;
    }

    fn keys_for_epoch(&self, epoch: u8) -> Option<&EpochKeys> {
        self.key_slots[(epoch & 1) as usize].as_ref()
    }

    pub fn dispatch_from_parts(
        &self,
        session_seq: u64,
        epoch: u8,
        envelope_info: crate::v4::codec::envelope::EnvelopeInfo,
        envelope: &[u8; ENVELOPE_LEN],
        body: &[u8],
    ) {
        tracing::debug!(
            session_seq, epoch,
            body_len = body.len(),
            current_epoch = self.current_epoch,
            "bulk_recv: dispatch_from_parts entered"
        );

        let wire_len = ENVELOPE_LEN + body.len();
        let mut wire_buf = self.wire_pool.acquire(wire_len);
        {
            let inner = wire_buf.inner_mut();
            inner.extend_from_slice(envelope);
            inner.extend_from_slice(body);
        }

        let keys = match self.keys_for_epoch(epoch) {
            Some(k) => k.clone(),
            None => {
                tracing::error!(
                    session_seq, epoch,
                    current_epoch = self.current_epoch,
                    slot0_present = self.key_slots[0].is_some(),
                    slot1_present = self.key_slots[1].is_some(),
                    "bulk_recv: no keys for epoch — fail-closed, pushing Fatal"
                );
                if self.result_queue.try_push(
                    session_seq,
                    BulkDecryptResult::Fatal(RecvDispatchError::EnvelopeMacFailed),
                ).is_err() {
                    tracing::error!(session_seq, "bulk_recv: result_queue full on Fatal push — aborting");
                    let _ = std::io::Write::write_fmt(
                        &mut std::io::stderr(),
                        format_args!(
                            "FATAL: no-keys-for-epoch Fatal could not be delivered — result_queue full.\n\
                             session_seq={session_seq}. Aborting to prevent silent security bypass.\n"
                        ),
                    );
                    std::process::abort();
                }
                return;
            }
        };

        tracing::debug!(
            session_seq, epoch,
            "bulk_recv: keys resolved, spawning rayon decrypt worker"
        );
        self.dispatch_wire_buf(session_seq, wire_buf, keys, envelope_info);
    }

    fn dispatch_wire_buf(
        &self,
        session_seq: u64,
        wire_buf: WireBuf,
        keys: EpochKeys,
        envelope_info: crate::v4::codec::envelope::EnvelopeInfo,
    ) {
        let result_queue = Arc::clone(&self.result_queue);

        let plaintext_capacity = wire_buf.len()
            .saturating_sub(ENVELOPE_LEN)
            .saturating_sub(STREAM_HEADER_LEN)
            .saturating_sub(AEAD_TAG_LEN);
        let recv_buf = self.recv_pool.acquire(plaintext_capacity);

        self.decrypt_pool.spawn(move || {
            tracing::debug!(session_seq, "bulk_recv: decrypt worker entered");

            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let result = decrypt_frame(
                    &wire_buf, envelope_info, &keys.header_key, &keys.cipher, recv_buf,
                );
                drop(wire_buf);
                result
            }));

            match outcome {
                Ok(Ok(chunk)) => {
                    let seq = chunk.link_input.session_seq;
                    tracing::debug!(
                        session_seq = seq,
                        stream_id = chunk.stream_id,
                        chunk_index = chunk.chunk_index,
                        kind = ?chunk.kind,
                        plaintext_len = chunk.plaintext.len(),
                        "bulk_recv: decrypt OK — pushing Chunk to result_queue"
                    );
                    if result_queue.try_push(seq, BulkDecryptResult::Chunk(chunk)).is_err() {
                        tracing::error!(session_seq = seq, "bulk_recv: result_queue full — chunk dropped");
                    } else {
                        tracing::debug!(session_seq = seq, "bulk_recv: Chunk pushed to result_queue");
                    }
                }
                Ok(Err(e)) => {
                    tracing::error!(session_seq, error = %e, "bulk_recv: decrypt_frame failed — pushing Fatal");
                    if result_queue.try_push(session_seq, BulkDecryptResult::Fatal(e)).is_err() {
                        tracing::error!(session_seq, "bulk_recv: result_queue full on Fatal push — aborting");
                        let _ = std::io::Write::write_fmt(
                            &mut std::io::stderr(),
                            format_args!(
                                "FATAL: bulk decrypt Fatal error could not be delivered — result_queue full.\n\
                                 session_seq={session_seq}. Aborting to prevent silent security bypass.\n"
                            ),
                        );
                        std::process::abort();
                    }
                }
                Err(panic_payload) => {
                    let msg = panic_payload.downcast_ref::<&str>()
                        .copied()
                        .or_else(|| panic_payload.downcast_ref::<String>().map(|s| s.as_str()))
                        .unwrap_or("<non-string panic>");
                    tracing::error!(session_seq, panic = msg, "bulk_recv: decrypt worker panicked — pushing Fatal");
                    if result_queue.try_push(session_seq, BulkDecryptResult::Fatal(RecvDispatchError::WorkerPanic)).is_err() {
                        tracing::error!(session_seq, "bulk_recv: result_queue full on WorkerPanic push — aborting");
                        let _ = std::io::Write::write_fmt(
                            &mut std::io::stderr(),
                            format_args!(
                                "FATAL: bulk decrypt WorkerPanic could not be delivered — result_queue full.\n\
                                 session_seq={session_seq}. Aborting to prevent silent security bypass.\n"
                            ),
                        );
                        std::process::abort();
                    }
                }
            }
            tracing::debug!(session_seq, "bulk_recv: decrypt worker exiting");
        });
    }
}

fn decrypt_frame(
    wire: &[u8],
    env_info: crate::v4::codec::envelope::EnvelopeInfo,
    header_key: &[u8; 32],
    cipher: &FrameCipher,
    mut recv_buf: crate::v4::io::lane_channels::RecvBuf,
) -> Result<DecryptedChunk, RecvDispatchError> {
    use crate::v4::codec::header;

    let session_seq = env_info.session_seq;
    tracing::debug!(session_seq, wire_len = wire.len(), "decrypt_frame: entered");

    if wire.len() < ENVELOPE_LEN + STREAM_HEADER_LEN + AEAD_TAG_LEN {
        tracing::error!(session_seq, wire_len = wire.len(), "decrypt_frame: frame too short");
        return Err(RecvDispatchError::FrameTooShort { len: wire.len() });
    }

    let envelope_bytes: [u8; ENVELOPE_LEN] = wire[..ENVELOPE_LEN]
        .try_into()
        .map_err(|_| RecvDispatchError::FrameTooShort { len: wire.len() })?;

    let body = &wire[ENVELOPE_LEN..];

    let header_bytes: [u8; STREAM_HEADER_LEN] = body[..STREAM_HEADER_LEN]
        .try_into()
        .map_err(|_| RecvDispatchError::FrameTooShort { len: wire.len() })?;

    let header_info = header::parse_header(&header_bytes, header_key)
        .map_err(|e| {
            tracing::error!(
                session_seq,
                header_key_fp = %hex::encode(&header_key[..8]),
                error = ?e,
                "decrypt_frame: HeaderMAC verification failed"
            );
            RecvDispatchError::HeaderMacFailed
        })?;

    tracing::debug!(
        session_seq,
        stream_id = header_info.stream_id,
        chunk_index = header_info.chunk_index,
        kind = ?header_info.frame_kind,
        "decrypt_frame: header verified, decrypting AEAD"
    );

    let ciphertext_and_tag = &body[STREAM_HEADER_LEN..];
    let decrypted_len = cipher.open_into(
        header_info.nonce,
        &envelope_bytes,
        Some(&header_bytes),
        ciphertext_and_tag,
        recv_buf.buf_mut(),
    ).map_err(|_| {
        tracing::error!(
            session_seq,
            stream_id = header_info.stream_id,
            chunk_index = header_info.chunk_index,
            "decrypt_frame: AEAD open failed"
        );
        RecvDispatchError::AeadFailed
    })?;
    recv_buf.set_len(decrypted_len);

    let chunk_digest = *blake3::hash(&recv_buf).as_bytes();

    let envelope_hash = *blake3::hash(&envelope_bytes).as_bytes();
    let header_hash = *blake3::hash(&header_bytes).as_bytes();
    let ciphertext_hash = *blake3::hash(body).as_bytes();

    let link_input = LinkInput {
        session_seq: env_info.session_seq,
        envelope_hash,
        header_hash,
        ciphertext_hash,
    };

    tracing::debug!(
        session_seq,
        stream_id = header_info.stream_id,
        chunk_index = header_info.chunk_index,
        plaintext_len = decrypted_len,
        digest_prefix = %hex::encode(&chunk_digest[..8]),
        "decrypt_frame: complete"
    );

    let chunk = DecryptedChunk {
        stream_id: header_info.stream_id,
        chunk_index: header_info.chunk_index,
        plaintext: PlaintextBuf::Pooled(recv_buf),
        kind: header_info.frame_kind,
        header_flags: header_info.header_flags,
        link_input,
        chunk_digest,
    };

    Ok(chunk)
}
