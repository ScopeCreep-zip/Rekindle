//! Bulk send pipeline — rayon-parallel encryption, unordered wire emission.
//!
//! Rayon workers fill pre-allocated WireBuf from the wire buffer freelist,
//! wrap in BulkFrame::Pooled, and send directly to the write task.
//! When the write task drops the BulkFrame, the inner Vec returns to
//! the freelist. Zero allocator pressure in steady state.
//!
//! Safety: pool.spawn(), NEVER pool.scope(). Scope deadlocks.

use std::sync::Arc;

use rayon::ThreadPool;
use tokio::sync::{mpsc, oneshot};

use crate::v4::audit::chain::LinkInput;
use crate::v4::bulk::AuditQueue;
use crate::v4::codec::envelope::{self, EnvelopeInfo};
use crate::v4::codec::header::{self, StreamHeaderInfo};
use crate::v4::codec::stream::open as open_codec;
use crate::v4::io::control_loop::fin::PendingFin;
use crate::v4::wire::outbound::{OutboundFrame, SequencedOutbound};
use crate::v4::io::encode::FrameEncoder;
use crate::v4::io::lane_channels::{BulkFrame, WireBufPool};
use crate::v4::wire::clearance::Clearance;
use crate::v4::wire::constants::{
    AEAD_TAG_LEN, ENVELOPE_LEN, MAX_BODY_LEN_DATA, STREAM_HEADER_LEN, WIRE_VERSION,
};
use crate::v4::wire::envelope::flags as env_flags;
use crate::v4::wire::frame_class::FrameClass;
use crate::v4::wire::frame_kind::StreamKind;
use crate::v4::wire::lane::Lane;

/// Sends bulk payload data via the rayon parallel encryption pipeline.
#[derive(Clone)]
pub struct BulkSender {
    encoder: Arc<FrameEncoder>,
    encrypt_pool: Arc<ThreadPool>,
    /// BulkFrame → crossbeam → write task.
    bulk_wire_tx: crossbeam::channel::Sender<BulkFrame>,
    /// LinkInput → DispatchQueue → control loop for outbound audit chain.
    /// Lock-free push from rayon, TokioWake notification to control loop.
    audit_queue: AuditQueue,
    /// Sequenced frames → control loop with confirmation (STREAM_OPEN).
    sequenced_tx: mpsc::Sender<SequencedOutbound>,
    /// PendingFin → control loop (signals when to emit STREAM_FIN).
    pending_fin_tx: mpsc::Sender<PendingFin>,
    /// Wire buffer freelist.
    wire_pool: WireBufPool,
    /// Backpressure: limits in-flight rayon tasks to prevent unbounded
    /// wire buf + plaintext accumulation in the rayon job queue.
    inflight_sem: Arc<tokio::sync::Semaphore>,
}

impl BulkSender {
    pub fn new(
        encoder: Arc<FrameEncoder>,
        encrypt_pool: Arc<ThreadPool>,
        bulk_wire_tx: crossbeam::channel::Sender<BulkFrame>,
        audit_queue: AuditQueue,
        sequenced_tx: mpsc::Sender<SequencedOutbound>,
        pending_fin_tx: mpsc::Sender<PendingFin>,
        wire_pool: WireBufPool,
    ) -> Self {
        // Limit in-flight rayon tasks to 2× thread count. This bounds
        // the number of wire bufs + closures queued in rayon's job deque,
        // preventing unbounded memory growth when the dispatch loop runs
        // faster than rayon workers can encrypt.
        let max_inflight = encrypt_pool.current_num_threads() * 2;
        Self {
            encoder, encrypt_pool,
            bulk_wire_tx, audit_queue, sequenced_tx, pending_fin_tx,
            wire_pool,
            inflight_sem: Arc::new(tokio::sync::Semaphore::new(max_inflight)),
        }
    }

    /// Send a complete bulk payload on a specific stream_id.
    pub async fn send(
        &self,
        stream_id: u8,
        payload: &[u8],
        clearance: Clearance,
    ) -> Result<u32, BulkSendError> {
        let transfer_id = uuid::Uuid::now_v7();
        let chunk_size: usize = (MAX_BODY_LEN_DATA as usize) - STREAM_HEADER_LEN - AEAD_TAG_LEN;
        let chunk_count = payload.chunks(chunk_size).count() as u32;

        // Merkle content hash — computed off the tokio thread to avoid blocking
        // the executor during the ~1.6ms BLAKE3 hash of a 16 MiB payload.
        // payload.to_vec() copies the payload (0.8ms at DDR4 bandwidth) so the
        // rayon closure owns the data ('static requirement). Net tokio thread
        // time saved: 0.8ms per frame at 120fps = 96ms/sec.
        let payload_owned = payload.to_vec();
        let hash_chunk_size = chunk_size;
        let hash_stream_id = stream_id;
        let (hash_tx, hash_rx) = tokio::sync::oneshot::channel();
        self.encrypt_pool.spawn(move || {
            tracing::debug!(stream_id = hash_stream_id, "bulk_send: hash worker entered");
            let mut content_hasher = blake3::Hasher::new();
            for (i, chunk) in payload_owned.chunks(hash_chunk_size).enumerate() {
                let digest = *blake3::hash(chunk).as_bytes();
                tracing::debug!(
                    stream_id = hash_stream_id, chunk_index = i, chunk_len = chunk.len(),
                    digest = %hex::encode(&digest[..8]),
                    "bulk_send: sender chunk digest"
                );
                content_hasher.update(&digest);
            }
            let root = *content_hasher.finalize().as_bytes();
            tracing::debug!(stream_id = hash_stream_id, root = %hex::encode(&root[..8]), "bulk_send: hash worker complete — Merkle root sent");
            let _ = hash_tx.send(root);
        });
        let content_hash = hash_rx.await.map_err(|_| BulkSendError::ConnectionLost)?;
        tracing::debug!(
            stream_id, chunk_count,
            content_hash = %hex::encode(&content_hash[..8]),
            "bulk_send: sender Merkle root (computed off-tokio-thread)"
        );

        tracing::info!(stream_id, chunk_count, total_bytes = payload.len(), "bulk_send: starting");

        // ── Step 1: STREAM_OPEN → control loop (sequenced) ──────────
        let open_payload = open_codec::encode(&open_codec::StreamOpenPayload {
            transfer_id,
            expected_total_bytes: payload.len() as u64,
            expected_chunk_count: chunk_count,
            chunk_size: chunk_size as u32,
            content_hash,
            lineage_kind: 0x01,
            dedup_hint: 0x03,
            clearance_required: clearance,
            conditions: vec![],
        });

        let (confirm_tx, confirm_rx) = oneshot::channel();
        self.sequenced_tx.send(SequencedOutbound {
            frame: OutboundFrame::Data {
                stream_id,
                kind: StreamKind::Open,
                chunk_index: 0,
                payload: open_payload,
            },
            confirm: confirm_tx,
            completion: None,
        }).await.map_err(|e| {
            tracing::error!(stream_id, "bulk_send: sequenced_tx.send failed (STREAM_OPEN) — control loop receiver dropped: {e}");
            BulkSendError::ConnectionLost
        })?;
        tracing::debug!(stream_id, "bulk_send: STREAM_OPEN sent to sequenced_tx, awaiting confirmation");

        confirm_rx.await.map_err(|e| {
            tracing::error!(stream_id, "bulk_send: confirm_rx failed (STREAM_OPEN) — control loop dropped oneshot without confirming: {e}");
            BulkSendError::ConnectionLost
        })?;
        tracing::debug!(stream_id, "bulk_send: STREAM_OPEN confirmed");

        // ── Step 2: Spawn rayon workers for each chunk ──────────────
        let mut last_chunk_seq: u64 = 0;

        // Clone senders once before the loop — avoids per-chunk atomic
        // refcount increment (LOCK XADD on x86, LDADD on ARM) inside the
        // hot dispatch path.
        let wire_tx = self.bulk_wire_tx.clone();
        let audit_queue = Arc::clone(&self.audit_queue);

        for (i, chunk) in payload.chunks(chunk_size).enumerate() {
            let cidx = i as u32;

            let seq = self.encoder.next_session_seq_atomic();
            last_chunk_seq = seq;

            tracing::debug!(
                stream_id,
                chunk_index = cidx,
                session_seq = seq,
                chunk_len = chunk.len(),
                "bulk_send: allocated session_seq for chunk"
            );

            let plaintext_len = chunk.len();
            let body_len = STREAM_HEADER_LEN + plaintext_len + AEAD_TAG_LEN;
            let wire_len = ENVELOPE_LEN + body_len;

            // Snapshot epoch + keys atomically at dispatch time.
            // The closure captures this snapshot — immutable for its lifetime.
            // If rotation happens between dispatch and execution, the worker
            // uses the epoch that was current at dispatch time. Both epochs
            // are valid during the transition window.
            let snap = self.encoder.snapshot();
            let epoch_flag = if snap.epoch & 1 == 1 { env_flags::KEY_EPOCH } else { 0 };

            let env_info = EnvelopeInfo {
                wire_version: WIRE_VERSION,
                lane: Lane::Data,
                flags: epoch_flag,
                body_len: u32::try_from(body_len).expect("body exceeds u32"),
                session_seq: seq,
            };
            let envelope_bytes = envelope::build_envelope(&env_info, &snap.keys.envelope_key);

            // FIN_FOLLOWS: for single-chunk transfers, set the flag on the
            // only payload frame. The receiver verifies and ACKs immediately
            // without waiting for a separate STREAM_FIN frame.
            let is_last_chunk = cidx == chunk_count - 1;
            let header_flags = if chunk_count == 1 && is_last_chunk {
                crate::v4::wire::header::flags::FIN_FOLLOWS
            } else {
                0
            };

            let header_info = StreamHeaderInfo {
                frame_class: FrameClass::Stream,
                frame_kind: StreamKind::Payload,
                stream_id,
                header_flags,
                chunk_index: cidx,
                nonce: seq,
            };
            let header_bytes = header::build_header(&header_info, &snap.keys.header_key);

            let cipher = snap.keys.cipher.clone();
            let chunk_wire_tx = wire_tx.clone();
            let chunk_audit_queue = Arc::clone(&audit_queue);

            // Backpressure: wait for a rayon slot before acquiring the wire
            // buf. This bounds the number of wire bufs held in the rayon
            // queue to inflight_sem permits. Without this, the dispatch loop
            // acquires all N wire bufs before any worker runs.
            tracing::debug!(
                stream_id, chunk_index = cidx, session_seq = seq,
                "bulk_send: acquiring inflight permit"
            );
            let permit = Arc::clone(&self.inflight_sem)
                .acquire_owned()
                .await
                .map_err(|e| {
                    tracing::error!(stream_id, chunk = i, "bulk_send: inflight_sem closed — semaphore dropped: {e}");
                    BulkSendError::ConnectionLost
                })?;
            tracing::debug!(
                stream_id, chunk_index = cidx, session_seq = seq,
                "bulk_send: permit acquired, building wire buf"
            );

            // Write envelope + header + plaintext into the wire buf on the
            // tokio thread. The rayon worker encrypts in-place and reads
            // AAD directly from wire_buf — no redundant captured copies.
            tracing::debug!(
                stream_id, chunk_index = cidx, session_seq = seq,
                wire_len,
                "bulk_send: acquiring wire buf from pool"
            );
            let mut wire_buf = self.wire_pool.acquire(wire_len);
            {
                let wire = wire_buf.inner_mut();
                wire.extend_from_slice(&envelope_bytes);
                wire.extend_from_slice(&header_bytes);
                wire.extend_from_slice(chunk);
            }
            tracing::debug!(
                stream_id, chunk_index = cidx, session_seq = seq,
                "bulk_send: wire buf built, spawning rayon encrypt worker"
            );

            self.encrypt_pool.spawn(move || {
                tracing::debug!(
                    stream_id, chunk_index = cidx, session_seq = seq,
                    plaintext_len,
                    "bulk_send: encrypt worker entered"
                );
                let wire = wire_buf.inner_mut();
                tracing::debug!(
                    stream_id, chunk_index = cidx, session_seq = seq,
                    wire_len = wire.len(),
                    "bulk_send: wire_buf.inner_mut() OK, building AAD slices"
                );
                let ct_offset = ENVELOPE_LEN + STREAM_HEADER_LEN;

                let env_aad: [u8; ENVELOPE_LEN] = wire[..ENVELOPE_LEN]
                    .try_into().expect("envelope slice");
                let hdr_aad: [u8; STREAM_HEADER_LEN] = wire[ENVELOPE_LEN..ENVELOPE_LEN + STREAM_HEADER_LEN]
                    .try_into().expect("header slice");

                tracing::debug!(
                    stream_id, chunk_index = cidx, session_seq = seq,
                    plaintext_len, ct_offset, wire_len,
                    "bulk_send: about to call seal_in_place"
                );
                let seal_start = std::time::Instant::now();
                let tag = cipher.seal_in_place(
                    seq, &env_aad, Some(&hdr_aad),
                    &mut wire[ct_offset..ct_offset + plaintext_len],
                );
                let seal_us = seal_start.elapsed().as_micros() as u64;
                tracing::debug!(
                    stream_id, chunk_index = cidx, session_seq = seq,
                    seal_us, plaintext_len,
                    "bulk_send: seal_in_place returned"
                );
                wire.extend_from_slice(&tag);
                assert_eq!(wire.len(), wire_len, "wire frame length mismatch after seal");
                tracing::debug!(
                    stream_id, chunk_index = cidx, session_seq = seq,
                    seal_us,
                    "bulk_send: seal_in_place complete, tag appended"
                );

                // Audit hashes from the final wire bytes (post-encryption).
                tracing::debug!(
                    stream_id, chunk_index = cidx, session_seq = seq,
                    "bulk_send: computing BLAKE3 audit hashes"
                );
                let hash_start = std::time::Instant::now();
                let envelope_hash = *blake3::hash(&wire[..ENVELOPE_LEN]).as_bytes();
                let header_hash = *blake3::hash(
                    &wire[ENVELOPE_LEN..ENVELOPE_LEN + STREAM_HEADER_LEN]
                ).as_bytes();
                let ciphertext_hash = *blake3::hash(&wire[ENVELOPE_LEN..]).as_bytes();
                let hash_us = hash_start.elapsed().as_micros() as u64;
                tracing::debug!(
                    stream_id, chunk_index = cidx, session_seq = seq,
                    hash_us, wire_body_len = wire.len() - ENVELOPE_LEN,
                    "bulk_send: BLAKE3 audit hashes complete"
                );

                let link_input = LinkInput {
                    session_seq: seq,
                    envelope_hash,
                    header_hash,
                    ciphertext_hash,
                };

                // Release permit BEFORE channel sends. CPU work (seal + hash)
                // is complete. Holding the permit during try_send would block
                // new dispatches while waiting on channel capacity.
                drop(permit);
                tracing::debug!(
                    stream_id, chunk_index = cidx, session_seq = seq,
                    "bulk_send: permit released"
                );

                // Audit link MUST be pushed before wire frame. If the queue
                // is full (structurally impossible when capacity >= inflight
                // permits), the wire frame is NOT sent — the chunk is dropped,
                // the receiver's reassembler detects the gap, and the session
                // terminates via content hash mismatch at FIN verification.
                crate::v4::bulk::counters::DIAG_AUDIT_OUTBOUND_PUSHES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let audit_result = chunk_audit_queue.try_push(seq, link_input);
                if audit_result.is_err() {
                    crate::v4::bulk::counters::DIAG_AUDIT_OUTBOUND_PUSH_FAILURES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    tracing::error!(stream_id, chunk_index = cidx, session_seq = seq, "bulk_send: audit link push FAILED — queue full");
                    return;
                }
                tracing::debug!(stream_id, chunk_index = cidx, session_seq = seq, "bulk_send: audit link pushed OK");

                let wire_result = chunk_wire_tx.try_send(BulkFrame::Pooled(wire_buf));
                if wire_result.is_err() {
                    tracing::error!(stream_id, chunk_index = cidx, session_seq = seq, "bulk_send: wire frame send FAILED — channel closed");
                    return;
                }
                tracing::debug!(stream_id, chunk_index = cidx, session_seq = seq, "bulk_send: wire frame sent to write task");
            });
        }

        // ── Step 3: PendingFin → control loop (multi-chunk only) ────
        // Single-chunk transfers use FIN_FOLLOWS — no separate FIN frame,
        // no PendingFin, no emit_stream_fin. The receiver verifies on
        // PAYLOAD arrival and ACKs immediately.
        if chunk_count > 1 {
            self.pending_fin_tx.send(PendingFin {
                stream_id,
                chunk_count,
                total_bytes: payload.len() as u64,
                content_hash,
                bulk_wire_tx: self.bulk_wire_tx.clone(),
                last_chunk_seq,
            }).await.map_err(|e| {
                tracing::error!(stream_id, chunk_count, "bulk_send: pending_fin_tx.send failed — control loop receiver dropped: {e}");
                BulkSendError::ConnectionLost
            })?;

            tracing::debug!(stream_id, chunk_count, last_chunk_seq, "bulk_send: PendingFin sent to Data lane");
        } else {
            tracing::debug!(stream_id, "bulk_send: single-chunk FIN_FOLLOWS — no PendingFin");
        }

        tracing::debug!(stream_id, chunk_count, last_chunk_seq, "bulk_send: send() returning OK");
        Ok(chunk_count)
    }
}

#[derive(Debug)]
pub enum BulkSendError {
    ConnectionLost,
}
