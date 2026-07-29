//! Async read task — reads frames from the socket, routes to either
//! inline FrameDecoder (small frames) or BulkReceiver (large Data lane frames).
//!
//! Every frame: read 32-byte Envelope → verify EMAC → replay filter → read body.
//! Then route:
//!   - Data lane AND body_len >= bulk_threshold → BulkReceiver::dispatch (rayon)
//!   - Everything else → FrameDecoder::decode_body inline → VerifiedFrame to control loop
//!
//! This task has NO select! — structurally cancel-safe. It does NOT own
//! per-lane state, does NOT dispatch application frames, does NOT run timers.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use tokio::io::AsyncReadExt;
use tokio::net::unix::OwnedReadHalf;
use tokio::sync::mpsc;

use crate::v4::bulk::counters::BulkCounters;
use crate::v4::bulk::recv::BulkReceiver;
use crate::v4::io::decode::{DecodeError, FrameDecoder};
use crate::v4::wire::constants::ENVELOPE_LEN;
use crate::v4::wire::lane::Lane;

// blake3 used for audit hash computation in the async fallback path

use super::control_loop::{ReadSignal, VerifiedFrame};

#[derive(Debug)]
pub enum SessionOutcome {
    Closed { peer_initiated: bool },
    HeartbeatTimeout { last_ping_at: u64 },
    EnvelopeMacFailed { session_seq: u64 },
    HeaderMacFailed { session_seq: u64 },
    AeadVerificationFailed { session_seq: u64 },
    AuditChainDivergence { checkpoint_seq: u64 },
    SubstrateReadFailed { detail: String },
    ConnectionLost,
    QuiescenceTimeout { duration_ms: u64 },
    RotationTimeout,
    ChannelError { code: u16, message: String },
    DrainTimeout { local_goodbye_sent: bool, peer_goodbye_received: bool, active_streams: usize },
    NonceExhausted,
    WireVersionUnsupported { received: u8 },
    LaneUnknown { received: u8 },
    ReservedBitSet { flags: u16 },
    SequenceNonMonotonic { expected: u64, received: u64 },
    FrameTooLarge { lane: Lane, body_len: u32 },
    FrameMalformed { lane: Lane, body_len: u32 },
    ReplayDetected { session_seq: u64 },
}

pub(crate) fn decode_error_to_outcome(e: DecodeError, fallback_seq: u64) -> SessionOutcome {
    match e {
        DecodeError::EnvelopeMacFailed => SessionOutcome::EnvelopeMacFailed { session_seq: fallback_seq },
        DecodeError::WireVersionUnsupported(v) => SessionOutcome::WireVersionUnsupported { received: v },
        DecodeError::LaneUnknown(v) => SessionOutcome::LaneUnknown { received: v },
        DecodeError::ReservedBitSet(f) => SessionOutcome::ReservedBitSet { flags: f },
        DecodeError::SequenceNonMonotonic { expected, received } => {
            SessionOutcome::SequenceNonMonotonic { expected, received }
        }
        DecodeError::FrameTooLarge { lane, body_len, .. } => {
            SessionOutcome::FrameTooLarge { lane, body_len }
        }
        DecodeError::FrameMalformed { lane, body_len, .. } => {
            SessionOutcome::FrameMalformed { lane, body_len }
        }
        DecodeError::HeaderMacFailed => SessionOutcome::HeaderMacFailed { session_seq: fallback_seq },
        DecodeError::AeadVerificationFailed => {
            SessionOutcome::AeadVerificationFailed { session_seq: fallback_seq }
        }
        DecodeError::BodyTooShort { lane, .. } => {
            SessionOutcome::FrameMalformed { lane, body_len: 0 }
        }
        DecodeError::ReplayDetected { session_seq } => {
            SessionOutcome::ReplayDetected { session_seq }
        }
        DecodeError::UnknownEpoch { .. } => {
            SessionOutcome::AeadVerificationFailed { session_seq: fallback_seq }
        }
    }
}

/// Run the read task.
pub async fn run(
    mut reader: OwnedReadHalf,
    mut decoder: FrameDecoder,
    signal_tx: mpsc::Sender<ReadSignal>,
    bulk_receiver: BulkReceiver,
    bulk_threshold: u32,
    counters: Arc<BulkCounters>,
    credit_guard: Arc<rekindle_transport_buff::CreditGuard>,
    last_activity_ns: Arc<std::sync::atomic::AtomicU64>,
) {
    let mut envelope_buf = [0u8; ENVELOPE_LEN];

    loop {
        // Read 32-byte Envelope
        match reader.read_exact(&mut envelope_buf).await {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                let _ = signal_tx.send(ReadSignal::Finished(SessionOutcome::ConnectionLost)).await;
                return;
            }
            Err(e) => {
                let _ = signal_tx.send(ReadSignal::Finished(
                    SessionOutcome::SubstrateReadFailed { detail: e.to_string() }
                )).await;
                return;
            }
        }

        // Verify EMAC + replay filter — before any field is consulted
        let (env_info, peer_epoch_advanced) = match decoder.verify_envelope(&envelope_buf) {
            Ok(result) => result,
            Err(DecodeError::ReplayDetected { session_seq }) => {
                counters.replay_rejections.fetch_add(1, Ordering::Relaxed);
                let _ = signal_tx.send(ReadSignal::Finished(
                    SessionOutcome::ReplayDetected { session_seq }
                )).await;
                return;
            }
            Err(e) => {
                let _ = signal_tx.send(ReadSignal::Finished(decode_error_to_outcome(e, 0))).await;
                return;
            }
        };

        // Read body
        let body_len = env_info.body_len as usize;
        let mut body = vec![0u8; body_len];
        match reader.read_exact(&mut body).await {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                let _ = signal_tx.send(ReadSignal::Finished(SessionOutcome::ConnectionLost)).await;
                return;
            }
            Err(e) => {
                let _ = signal_tx.send(ReadSignal::Finished(
                    SessionOutcome::SubstrateReadFailed { detail: e.to_string() }
                )).await;
                return;
            }
        }

        // Record received frame in counters
        counters.record_received(1, (ENVELOPE_LEN + body_len) as u64);
        // Record activity at the read task level — visible to the heartbeat
        // even when the control loop is blocked in drain_outbound AEAD.
        let now_ns = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0);
        last_activity_ns.store(now_ns, std::sync::atomic::Ordering::Release);

        // Route: Data lane + large body → BulkReceiver (rayon parallel decrypt)
        //        Everything else → FrameDecoder inline decrypt → control loop
        if env_info.lane == Lane::Data && env_info.body_len >= bulk_threshold {
            let frame_len = (ENVELOPE_LEN + body_len) as u64;
            if !credit_guard.try_reserve(frame_len) {
                counters.memory_pressure_drops.fetch_add(1, Ordering::Relaxed);
                tracing::warn!(
                    session_seq = env_info.session_seq,
                    frame_len,
                    inflight = credit_guard.inflight(),
                    ceiling = credit_guard.ceiling(),
                    "CreditGuard: frame shed under memory pressure"
                );
                continue;
            }
            bulk_receiver.dispatch_from_parts(
                env_info.session_seq,
                env_info.key_epoch(),
                env_info,
                &envelope_buf,
                &body,
            );
            continue;
        }

        // Inline decrypt for small frames and non-Data lanes
        let decoded = match decoder.decode_body(&envelope_buf, &env_info, &body) {
            Ok(d) => d,
            Err(e) => {
                if matches!(e, DecodeError::AeadVerificationFailed) {
                    counters.aead_failures.fetch_add(1, Ordering::Relaxed);
                }
                let _ = signal_tx.send(ReadSignal::Finished(
                    decode_error_to_outcome(e, env_info.session_seq)
                )).await;
                return;
            }
        };

        // Compute audit hashes while body is in cache
        let audit_envelope_hash = *blake3::hash(&envelope_buf).as_bytes();
        let audit_header_hash = if env_info.lane == Lane::Data
            && body_len >= crate::v4::wire::constants::STREAM_HEADER_LEN
        {
            *blake3::hash(&body[..crate::v4::wire::constants::STREAM_HEADER_LEN]).as_bytes()
        } else {
            [0u8; 32]
        };
        let audit_ciphertext_hash = *blake3::hash(&body).as_bytes();

        // Retention bytes
        let mut retained_wire = Vec::with_capacity(ENVELOPE_LEN + body_len);
        retained_wire.extend_from_slice(&envelope_buf);
        retained_wire.extend_from_slice(&body);

        let frame = VerifiedFrame {
            envelope_bytes: envelope_buf,
            envelope: decoded.envelope,
            header: decoded.header,
            plaintext: decoded.plaintext,
            peer_epoch_advanced,
            audit_envelope_hash,
            audit_header_hash,
            audit_ciphertext_hash,
            retained_wire,
        };

        if signal_tx.send(ReadSignal::Frame(frame)).await.is_err() {
            return;
        }
    }
}
