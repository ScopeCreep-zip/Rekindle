//! Bulk receive pipeline — rayon-parallel decryption with reassembly.
//!
//! The read task dispatches large Data lane frames to `BulkReceiver` for
//! parallel AEAD decryption on the rayon pool. Rayon workers decrypt into
//! pre-allocated RecvBuf from the recv buffer freelist — zero allocations
//! on rayon worker threads. Results are pushed into a DispatchQueue for
//! the control loop to consume via TokioWake notification.
//!
//! Epoch-aware: after key rotation, the BulkReceiver's key slots are
//! updated via `install_epoch_keys` (called by the read task's
//! `apply_epoch_signal` helper). The rayon closure captures the correct
//! epoch's keys at dispatch time — same snapshot pattern as the encoder.

use std::sync::Arc;

use rayon::ThreadPool;

use rekindle_transport_buff::adapters::tokio::TokioWake;
use rekindle_transport_buff::DispatchQueue;

use crate::v3::audit::chain::LinkInput;
use crate::v3::bulk::AuditQueue;
use crate::v3::codec::aead::FrameCipher;
use crate::v3::io::encode::EpochKeys;
use crate::v3::io::lane_channels::{PlaintextBuf, RecvBufPool, WireBuf, WireBufPool};
use crate::v3::wire::constants::{AEAD_TAG_LEN, ENVELOPE_LEN, STREAM_HEADER_LEN};
use crate::v3::wire::frame_kind::StreamKind;

/// A decrypted chunk ready for reassembly, OR a fatal error from a rayon worker.
pub enum BulkDecryptResult {
    /// Successfully decrypted chunk — insert into reassembler.
    Chunk(DecryptedChunk),
    /// Fatal decryption error — connection must be torn down.
    Fatal(RecvDispatchError),
}

/// A decrypted chunk ready for reassembly.
pub struct DecryptedChunk {
    pub stream_id: u8,
    pub chunk_index: u32,
    /// Decrypted plaintext. Pooled buffers return to RecvBufPool on Drop.
    pub plaintext: PlaintextBuf,
    /// The StreamKind from the decoded header — used by the control loop
    /// to dispatch on Fin vs Payload without payload-length heuristics.
    pub kind: StreamKind,
    /// Header flags from the stream header. Carries FIN_FOLLOWS for
    /// single-chunk transfers — the control loop checks this to trigger
    /// immediate content hash verification without a separate FIN frame.
    pub header_flags: u8,
    pub link_input: LinkInput,
    /// BLAKE3 digest of plaintext, computed on the rayon worker while
    /// data is in L1 cache. Fed into MerkleDigest for whole-transfer
    /// verification. Zero additional memory-bandwidth cost.
    pub chunk_digest: [u8; 32],
}

/// Dispatch errors from the bulk receive pipeline.
#[derive(Debug)]
pub enum RecvDispatchError {
    FrameTooShort { len: usize },
    EnvelopeMacFailed,
    HeaderMacFailed,
    AeadFailed,
    /// Rayon decrypt worker panicked — session must be terminated.
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

/// Receives inbound bulk Data lane frames, dispatches to rayon for
/// parallel AEAD decryption, and delivers decrypted chunks to the
/// control loop via DispatchQueue + TokioWake.
///
/// Epoch-aware: holds up to 2 key sets indexed by epoch & 1, identical
/// to the FrameDecoder's DecoderKeySlots. Updated by the read task via
/// `install_epoch_keys` when an epoch install signal arrives.
pub struct BulkReceiver {
    decrypt_pool: Arc<ThreadPool>,
    /// Results delivered to the control loop. Carries (session_seq, result).
    result_queue: Arc<DispatchQueue<BulkDecryptResult, TokioWake>>,
    /// Audit LinkInputs → DispatchQueue → control loop for inbound audit chain.
    audit_queue: AuditQueue,
    /// Epoch-aware key slots — at most 2 key sets indexed by epoch & 1.
    /// Updated by install_epoch_keys. Retirement is slot overwrite.
    key_slots: [Option<EpochKeys>; 2],
    current_epoch: u8,
    /// Plaintext buffer freelist. Buffers are acquired before rayon spawn
    /// and returned to the pool when the application drops the chunk.
    recv_pool: RecvBufPool,
    /// Wire buffer freelist for inbound ciphertext frames.
    wire_pool: WireBufPool,
}

impl BulkReceiver {
    pub fn new(
        envelope_key: [u8; 32],
        header_key: [u8; 32],
        cipher: Arc<FrameCipher>,
        decrypt_pool: Arc<ThreadPool>,
        result_queue: Arc<DispatchQueue<BulkDecryptResult, TokioWake>>,
        audit_queue: AuditQueue,
        recv_pool: RecvBufPool,
        wire_pool: WireBufPool,
    ) -> Self {
        // Initialize epoch=0 key slot from handshake keys.
        let initial_keys = EpochKeys {
            envelope_key,
            header_key,
            cipher: (*cipher).clone(),
        };
        let mut key_slots: [Option<EpochKeys>; 2] = [None, None];
        key_slots[0] = Some(initial_keys);

        Self {
            decrypt_pool, result_queue, audit_queue,
            key_slots,
            current_epoch: 0,
            recv_pool, wire_pool,
        }
    }

    /// Current epoch — for debug_assert synchronization with FrameDecoder.
    pub fn current_epoch(&self) -> u8 { self.current_epoch }

    /// Access the result queue's TokioWake for the control loop to await.
    pub fn wake(&self) -> &TokioWake {
        self.result_queue.wake_sink()
    }

    /// Install new epoch keys. Called by the read task's apply_epoch_signal
    /// helper when an epoch install signal arrives.
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

    // Retirement is slot overwrite — no explicit retire method needed.
    // install_epoch_keys(epoch, keys) drops the previous slot occupant
    // (epoch-2 keys) via the `= Some(keys)` assignment. ZeroizeOnDrop fires.

    /// Select keys for a given epoch. Returns None if the epoch has no
    /// installed keys — the frame must be rejected (fail-closed).
    fn keys_for_epoch(&self, epoch: u8) -> Option<&EpochKeys> {
        self.key_slots[(epoch & 1) as usize].as_ref()
    }

    /// Dispatch from envelope + body slices with the epoch already extracted
    /// by the caller (FrameDecoder.verify_envelope). The epoch is MAC-verified
    /// at that point — the BulkReceiver never reads raw flag bytes.
    pub fn dispatch_from_parts(&self, session_seq: u64, epoch: u8, envelope: &[u8; ENVELOPE_LEN], body: &[u8]) {
        let wire_len = ENVELOPE_LEN + body.len();
        let mut wire_buf = self.wire_pool.acquire(wire_len);
        {
            let inner = wire_buf.inner_mut();
            inner.extend_from_slice(envelope);
            inner.extend_from_slice(body);
        }

        tracing::trace!(
            session_seq, epoch,
            current_epoch = self.current_epoch,
            "BulkReceiver: dispatching frame with epoch-aware keys"
        );

        let keys = match self.keys_for_epoch(epoch) {
            Some(k) => k.clone(),
            None => {
                tracing::error!(
                    session_seq, epoch,
                    current_epoch = self.current_epoch,
                    slot0_present = self.key_slots[0].is_some(),
                    slot1_present = self.key_slots[1].is_some(),
                    "BulkReceiver: no keys for epoch — dropping frame (fail-closed)"
                );
                let _ = self.result_queue.try_push(
                    session_seq,
                    BulkDecryptResult::Fatal(RecvDispatchError::EnvelopeMacFailed),
                );
                return;
            }
        };

        self.dispatch_wire_buf(session_seq, wire_buf, keys);
    }

    /// Dispatch a pooled wire buffer for parallel decryption with the
    /// given epoch key snapshot.
    fn dispatch_wire_buf(&self, session_seq: u64, wire_buf: WireBuf, keys: EpochKeys) {
        let audit_queue = Arc::clone(&self.audit_queue);
        let result_queue = Arc::clone(&self.result_queue);

        // Pre-allocate the plaintext buffer on the dispatching thread (read
        // task). The rayon worker decrypts into it via open_into — no Vec<u8>
        // allocation on the worker thread, no glibc arena retention.
        let plaintext_capacity = wire_buf.len()
            .saturating_sub(ENVELOPE_LEN)
            .saturating_sub(STREAM_HEADER_LEN)
            .saturating_sub(AEAD_TAG_LEN);
        let recv_buf = self.recv_pool.acquire(plaintext_capacity);

        self.decrypt_pool.spawn(move || {
            // catch_unwind prevents a panicking decrypt worker from triggering
            // process::abort() via rayon's AbortIfPanic guard. Instead, we push
            // Fatal(WorkerPanic) so the control loop terminates the session cleanly.
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let result = decrypt_frame(
                    &wire_buf, &keys.envelope_key, &keys.header_key, &keys.cipher, recv_buf,
                );
                drop(wire_buf);
                result
            }));

            match outcome {
                Ok(Ok((chunk, link_input))) => {
                    crate::v3::bulk::counters::DIAG_AUDIT_INBOUND_PUSHES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    if audit_queue.try_push(link_input.session_seq, link_input).is_err() {
                        crate::v3::bulk::counters::DIAG_AUDIT_INBOUND_PUSH_FAILURES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        tracing::error!(session_seq, "inbound audit queue full — link dropped");
                    }
                    let _ = result_queue.try_push(link_input.session_seq, BulkDecryptResult::Chunk(chunk));
                }
                Ok(Err(e)) => {
                    let _ = result_queue.try_push(session_seq, BulkDecryptResult::Fatal(e));
                }
                Err(panic_payload) => {
                    let msg = panic_payload.downcast_ref::<&str>()
                        .copied()
                        .or_else(|| panic_payload.downcast_ref::<String>().map(|s| s.as_str()))
                        .unwrap_or("<non-string panic>");
                    tracing::error!(session_seq, panic = msg, "bulk decrypt worker panicked — session will terminate");
                    let _ = result_queue.try_push(session_seq, BulkDecryptResult::Fatal(RecvDispatchError::WorkerPanic));
                }
            }
        });
    }
}

/// Decrypt a single wire frame on a rayon worker thread.
/// Decrypts into the pre-allocated RecvBuf — zero heap allocation on this thread.
fn decrypt_frame(
    wire: &[u8],
    envelope_key: &[u8; 32],
    header_key: &[u8; 32],
    cipher: &FrameCipher,
    mut recv_buf: crate::v3::io::lane_channels::RecvBuf,
) -> Result<(DecryptedChunk, LinkInput), RecvDispatchError> {
    use crate::v3::codec::envelope;
    use crate::v3::codec::header;
    if wire.len() < ENVELOPE_LEN + STREAM_HEADER_LEN + 16 {
        return Err(RecvDispatchError::FrameTooShort { len: wire.len() });
    }

    let envelope_bytes: [u8; ENVELOPE_LEN] = wire[..ENVELOPE_LEN]
        .try_into()
        .map_err(|_| RecvDispatchError::FrameTooShort { len: wire.len() })?;

    // EMAC verification with epoch-correct envelope key
    let env_info = envelope::parse_envelope(&envelope_bytes, envelope_key)
        .map_err(|e| {
            tracing::error!(
                envelope_key_fp = %hex::encode(&envelope_key[..8]),
                error = ?e,
                "bulk decrypt_frame: EMAC verification failed on rayon worker"
            );
            RecvDispatchError::EnvelopeMacFailed
        })?;

    let body = &wire[ENVELOPE_LEN..];

    // HeaderMAC verification with epoch-correct header key
    let header_bytes: [u8; STREAM_HEADER_LEN] = body[..STREAM_HEADER_LEN]
        .try_into()
        .map_err(|_| RecvDispatchError::FrameTooShort { len: wire.len() })?;

    let header_info = header::parse_header(&header_bytes, header_key)
        .map_err(|e| {
            tracing::error!(
                header_key_fp = %hex::encode(&header_key[..8]),
                error = ?e,
                "bulk decrypt_frame: HeaderMAC verification failed on rayon worker"
            );
            RecvDispatchError::HeaderMacFailed
        })?;

    // AEAD verification + decryption into pre-allocated RecvBuf
    let ciphertext_and_tag = &body[STREAM_HEADER_LEN..];
    let ct_len = ciphertext_and_tag.len().saturating_sub(AEAD_TAG_LEN);
    let dest_len = recv_buf.buf_mut().len();
    tracing::trace!(
        wire_len = wire.len(),
        body_len = body.len(),
        ct_and_tag_len = ciphertext_and_tag.len(),
        ct_len,
        dest_len,
        "bulk_recv: decrypt_frame buffer sizes"
    );
    let decrypted_len = cipher.open_into(
        header_info.nonce,
        &envelope_bytes,
        Some(&header_bytes),
        ciphertext_and_tag,
        recv_buf.buf_mut(),
    ).map_err(|_| RecvDispatchError::AeadFailed)?;
    recv_buf.set_len(decrypted_len);

    // Compute chunk digest while plaintext is in L1 cache
    let chunk_digest = *blake3::hash(&recv_buf).as_bytes();
    tracing::trace!(
        stream_id = header_info.stream_id,
        chunk_index = header_info.chunk_index,
        kind = ?header_info.frame_kind,
        plaintext_len = decrypted_len,
        digest = %hex::encode(&chunk_digest[..8]),
        "bulk_recv: decrypted chunk digest"
    );

    // Compute audit chain hashes from the wire bytes
    let envelope_hash = *blake3::hash(&envelope_bytes).as_bytes();
    let header_hash = *blake3::hash(&header_bytes).as_bytes();
    let ciphertext_hash = *blake3::hash(body).as_bytes();

    let link_input = LinkInput {
        session_seq: env_info.session_seq,
        envelope_hash,
        header_hash,
        ciphertext_hash,
    };

    let chunk = DecryptedChunk {
        stream_id: header_info.stream_id,
        chunk_index: header_info.chunk_index,
        plaintext: PlaintextBuf::Pooled(recv_buf),
        kind: header_info.frame_kind,
        header_flags: header_info.header_flags,
        link_input,
        chunk_digest,
    };

    Ok((chunk, link_input))
}
