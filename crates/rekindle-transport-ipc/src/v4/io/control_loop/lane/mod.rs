//! Per-lane task module — four concrete run functions, one FrameLoop commodity.
//!
//! # Architecture
//!
//! ```text
//! read task (io_uring, std::thread)
//!   ├─► control::run  — Channel (0x01) + Datagram (0x03), heartbeat, lifecycle
//!   ├─► data::run     — Stream (0x02), bulk decrypt, FIN tracking
//!   ├─► audit::run    — Audit (0x04)
//!   └─► handoff::run  — Handoff/Streaming (0x05)
//!         │
//!         └─► audit_merge::run — collects LinkInputs, maintains chains
//! ```
//!
//! Each lane task owns its subsystem state exclusively. No sharing, no
//! locking on the per-frame hot path. Cross-lane state (session_state,
//! shutting_down, capabilities) is in `SharedSessionState` behind a
//! `parking_lot::RwLock`. Audit chain current links are in
//! `SharedAuditLinks` behind `AtomicU64` arrays (RCU pattern).
//!
//! # Commodity infrastructure
//!
//! `FrameLoop` owns only what ALL lanes need unconditionally: audit_tx,
//! lane_channels, encoder, small_encoder, and the reusable outbound
//! buffer. Router and ConnectionInfo are NOT in FrameLoop — the Audit
//! lane doesn't use them.
//!
//! `process_frame` is synchronous. It returns the audit LinkInput.
//! The caller does async sends after process_frame returns. This makes
//! it cancellation-safe inside tokio::select!.
//!
//! # Capacity model
//!
//! Target workload: 8 concurrent 4K NV12 120fps streams.
//!
//! Per-stream frame rate:
//! - 120 SharedMemRef (ArenaWrite) + 120 SlotRelease = 240 Handoff frames/sec
//! - Per 8 streams: 1920 Handoff frames/sec
//!
//! Per-stream data rate:
//! - 12,441,600 bytes/frame × 120fps = 1.49 GiB/s per stream
//! - Per 8 streams: 11.9 GiB/s total payload (shared memory, not socket)
//!
//! Control overhead at 8 streams:
//! - (65 + 57) × 120 × 8 = 117 KB/s wire bytes for SharedMemRef + SlotRelease
//! - 0.008% of data bandwidth
//!
//! Lane isolation guarantee:
//! - Key rotation (5ms AEAD rekey) on Control lane does NOT block
//!   Handoff lane's 1920 frames/sec. Zero frame drops during rotation.
//! - Bulk reassembly on Data lane does NOT block Handoff lane.
//! - Audit chain advancement on audit_merge does NOT block any lane —
//!   LinkInputs flow via async .send().await, chain links are read via
//!   AtomicU64 (SharedAuditLinks).
//!
//! IO bottleneck: `u->iolock` in `unix_stream_read_generic` serializes
//! all reads on one socket. The io_uring read task (single OS thread)
//! is the throughput ceiling. Lane sharding eliminates the PROCESSING
//! bottleneck (per-frame handler dispatch), not the IO bottleneck.
//! Estimated read task throughput: ~50K small frames/sec (256-byte
//! datagrams, i7-10875H, kernel 6.18, sk_sndbuf=256KiB).
//! At 1920 Handoff + ~200 Control + ~100 Audit = ~2220 inline frames/sec,
//! the read task uses ~4.4% of capacity.
//!
//! # Acceptance criteria
//!
//! 1. Zero frame drops during key rotation at 8×4K120 concurrent streams.
//!    Validated by: rotation handler on Control lane while Handoff lane
//!    processes ArenaWrite/SlotRelease without interruption.
//!
//! 2. Audit chain integrity under concurrent lane processing.
//!    Validated by: all 4 lanes send LinkInputs to audit_merge via
//!    .send().await (NEVER try_send). Merge task reorders by session_seq.
//!    Chain entries emitted in global order. Compare sharded chain output
//!    to sequential chain output — must be identical.
//!
//! 3. Session state transitions are atomic and visible to all lanes.
//!    Validated by: Control lane sets shutting_down=true in
//!    SharedSessionState. Data lane's next frame gate check rejects
//!    STREAM_OPEN. Handoff lane's next frame gate check rejects
//!    ArenaSetup. No stale state window.
//!
//! 4. Graceful shutdown drains all in-flight frames.
//!    Validated by: read task drops LaneInboundChannels → all lane
//!    receivers return None → lane tasks break → drop audit_merge_tx
//!    clones → merge task breaks → write task drains remaining
//!    outbound → socket closes.
//!
//! 5. Arena slot lifecycle correctness under concurrent streams.
//!    Validated by: SharedArena CAS state machine (FREE→ACQUIRED→
//!    IN_FLIGHT→FREE) is independent of lane task scheduling.
//!    SlotRelease processed by Handoff lane, return_slot CAS succeeds
//!    regardless of what Control/Data/Audit lanes are doing.
//!    reclaim_all_inflight on Handoff lane shutdown recovers leaked slots.
//!
//! 6. No handler can access state it doesn't declare in its signature.
//!    Validated by: compiler rejects any handler that references a
//!    subsystem not passed as a parameter. dispatch/inbound.rs is the
//!    single wiring point — adding a new handler requires adding one
//!    match arm with explicit subsystem references.

pub mod state;
pub mod handoff;
pub mod audit;
pub mod data;
pub mod control;

use std::sync::Arc;

use tokio::sync::mpsc;

use crate::v4::audit::chain::LinkInput;
use crate::v4::audit::retention::RetentionBuffer;
use crate::v4::handlers::HandlerError;
use crate::v4::io::encode::{FrameEncoder, SmallFrameEncoder};
use crate::v4::io::lane_channels::LaneChannels;
use crate::v4::wire::outbound::OutboundFrame;

use super::audit_merge::AuditLinkDirection;
use super::shared_state::SessionStateHandle;
use super::VerifiedFrame;

// ── LaneShutdown ────────────────────────────────────────────────

/// Returned when a channel is closed — the lane task must break.
#[must_use]
pub struct LaneShutdown;

// ── FrameLoop — commodity infrastructure ────────────────────────
//
// Owns only what ALL four lanes need unconditionally. Router and
// ConnectionInfo are NOT here — the Audit lane doesn't use them.
// Fields are private. process_frame is the only frame-processing API.

pub(crate) struct FrameLoop {
    shared: SessionStateHandle,
    audit_tx: mpsc::Sender<AuditLinkDirection>,
    lane_channels: LaneChannels,
    encoder: Arc<FrameEncoder>,
    small_encoder: SmallFrameEncoder,
    outbound: Vec<OutboundFrame>,
    retention: Arc<parking_lot::Mutex<RetentionBuffer>>,
}

impl FrameLoop {
    pub fn new(
        shared: SessionStateHandle,
        audit_tx: mpsc::Sender<AuditLinkDirection>,
        lane_channels: LaneChannels,
        encoder: Arc<FrameEncoder>,
        retention: Arc<parking_lot::Mutex<RetentionBuffer>>,
    ) -> Self {
        Self {
            shared,
            audit_tx,
            lane_channels,
            encoder,
            small_encoder: SmallFrameEncoder::new(),
            outbound: Vec::with_capacity(8),
            retention,
        }
    }

    /// Shared state handle — for lane tasks that need to clone it
    /// (e.g., Control lane passes it to dispatch_control for handlers
    /// that write session_state).
    pub fn shared(&self) -> &SessionStateHandle {
        &self.shared
    }

    /// Encoder reference — used by lane-specific operations that need
    /// to encode outbound frames outside process_frame (e.g., Data
    /// lane FIN emission, Control lane sequenced outbound).
    pub fn encoder(&self) -> &Arc<FrameEncoder> {
        &self.encoder
    }

    /// Audit merge sender — used by lane tasks that need to send
    /// outbound audit LinkInputs outside process_frame (e.g., Control
    /// lane sequenced outbound, Data lane FIN emission).
    pub fn audit_tx(&self) -> &mpsc::Sender<AuditLinkDirection> {
        &self.audit_tx
    }

    /// Lane channels — used by lane tasks that need to send encoded
    /// wire bytes outside process_frame.
    pub fn lane_channels(&self) -> &LaneChannels {
        &self.lane_channels
    }

    /// Process one frame: state gate → dispatch → return audit LinkInput.
    ///
    /// Synchronous. No .await points. The caller does async sends after
    /// this returns. Cancellation-safe inside tokio::select! — dropping
    /// the caller's future before the async sends means the dispatch
    /// never ran (process_frame already returned).
    ///
    /// The closure captures per-lane state by &mut and calls its lane's
    /// dispatch function. process_frame passes only the frame and the
    /// outbound buffer — no forced pass-through of router/info/header.
    pub fn process_frame<F>(
        &mut self,
        frame: &VerifiedFrame,
        dispatch: F,
    ) -> Result<LinkInput, HandlerError>
    where
        F: FnOnce(&VerifiedFrame, &mut Vec<OutboundFrame>) -> Result<(), HandlerError>,
    {
        // Clear at start — cancellation safety. If a previous call's
        // async drain was cancelled, stale frames are discarded here.
        self.outbound.clear();

        // State gate — read lock <100ns, dropped before dispatch.
        {
            let s = self.shared.read();
            if let Some((class, kind)) = extract_class_kind_for_gate(frame) {
                if let Ok(fc) = crate::v4::wire::frame_class::FrameClass::try_from(class) {
                    if !crate::v4::session::state::is_frame_allowed(s.session_state, fc, kind) {
                        return Err(HandlerError::FrameDisallowedInState);
                    }
                }
            }
        } // read lock dropped

        // Dispatch — closure captures per-lane state, calls dispatch fn
        dispatch(frame, &mut self.outbound)?;

        // Build inbound audit LinkInput from pre-computed hashes
        Ok(LinkInput {
            session_seq: frame.envelope.session_seq,
            envelope_hash: frame.audit_envelope_hash,
            header_hash: frame.audit_header_hash,
            ciphertext_hash: frame.audit_ciphertext_hash,
        })
    }

    /// Drain outbound frames through encoder → lane_channels.
    /// Sends audit LinkInputs for each outbound frame.
    /// Stores retained_wire for AUDIT_REPLAY.
    /// Async — called by the lane task after process_frame returns.
    pub async fn drain_and_emit_audit(
        &mut self,
        inbound_link: LinkInput,
        retained_wire: Vec<u8>,
    ) -> Result<(), LaneShutdown> {
        // Store retained wire for AUDIT_REPLAY before sending LinkInput
        {
            let mut ret = self.retention.lock();
            ret.store(inbound_link.session_seq, retained_wire);
        }

        // Send inbound audit LinkInput — NEVER try_send
        if self.audit_tx.capacity() == 0 {
            crate::v4::bulk::counters::DIAG_AUDIT_TX_BACKPRESSURE
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            tracing::warn!(
                session_seq = inbound_link.session_seq,
                "frame_loop: audit_tx backpressure — capacity at zero"
            );
        }
        tracing::trace!(
            session_seq = inbound_link.session_seq,
            "frame_loop: sending inbound audit link to audit_merge"
        );
        self.audit_tx
            .send(AuditLinkDirection::Inbound(inbound_link))
            .await
            .map_err(|_| LaneShutdown)?;

        // Encode and send each outbound frame
        let outbound_count = self.outbound.len();
        if outbound_count > 0 {
            tracing::debug!(
                outbound_count,
                "frame_loop: encoding outbound frames from process_frame"
            );
        }
        for frame_out in self.outbound.drain(..) {
            let lane = frame_out.lane();

            if frame_out.is_data_frame() {
                let ewa = self.encoder.encode_with_audit(&frame_out);
                tracing::debug!(
                    session_seq = ewa.link_input.session_seq,
                    lane = ?ewa.lane,
                    "frame_loop: outbound data frame encoded"
                );
                self.audit_tx
                    .send(AuditLinkDirection::Outbound(ewa.link_input))
                    .await
                    .map_err(|_| LaneShutdown)?;
                self.lane_channels
                    .send_with_retry(ewa.lane, ewa.encoded.into_wire_bytes())
                    .await
                    .map_err(|_| LaneShutdown)?;
            } else {
                let encoded = self.small_encoder.encode_small(
                    &self.encoder,
                    lane,
                    frame_out.class_byte(),
                    frame_out.kind_byte(),
                    frame_out.payload_bytes(),
                );
                let wire_bytes = encoded.wire.to_vec();
                let out_link = encoded.link_input;
                tracing::debug!(
                    session_seq = out_link.session_seq,
                    ?lane,
                    "frame_loop: outbound small frame encoded"
                );

                self.audit_tx
                    .send(AuditLinkDirection::Outbound(out_link))
                    .await
                    .map_err(|_| LaneShutdown)?;
                self.lane_channels
                    .send_with_retry(lane, wire_bytes)
                    .await
                    .map_err(|_| LaneShutdown)?;
            }
        }

        Ok(())
    }
}

// ── Helpers ─────────────────────────────────────────────────────

/// Extract (class, kind, handler_payload) from a verified frame.
/// Data lane: class/kind in stream header, payload is the full plaintext.
/// Other lanes: plaintext starts with [class, kind, ...], skip 2.
pub(crate) fn extract_class_kind(frame: &VerifiedFrame) -> Option<(u8, u8, &[u8])> {
    if let Some(ref h) = frame.header {
        Some((h.frame_class as u8, h.frame_kind as u8, frame.plaintext.as_slice()))
    } else if frame.plaintext.len() >= 2 {
        Some((frame.plaintext[0], frame.plaintext[1], &frame.plaintext[2..]))
    } else {
        None
    }
}

/// Extract (class, kind) for the state gate check only.
fn extract_class_kind_for_gate(frame: &VerifiedFrame) -> Option<(u8, u8)> {
    if let Some(ref h) = frame.header {
        Some((h.frame_class as u8, h.frame_kind as u8))
    } else if frame.plaintext.len() >= 2 {
        Some((frame.plaintext[0], frame.plaintext[1]))
    } else {
        None
    }
}
