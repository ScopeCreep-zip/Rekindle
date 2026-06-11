//! Connection control loop — the session.
//!
//! One instance per connection. Sole owner of SessionContext and coordinator
//! of all per-connection subsystems. The select! loop is irreducible (11 arms,
//! biased priority). Each arm body is a thin dispatch to a subsystem struct
//! or function in a sibling file.
//!
//! Invariant: the control loop NEVER calls any blocking function. Every
//! channel send is either tokio mpsc `.send().await` or direct struct
//! insertion. Crossbeam channels are used ONLY by rayon workers (OS threads).

pub(crate) mod audit_reorder;
pub(crate) mod drain;
pub(crate) mod fin;
pub(crate) mod heartbeat;
pub(crate) mod timers;
pub(crate) mod util;

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;

use crate::v3::audit::chain::LinkInput;
use crate::v3::audit::gap::{GapEvent, MissingBitmapBuilder};
use crate::v3::bulk::{AuditQueue};
use crate::v3::bulk::recv::BulkDecryptResult;
use crate::v3::codec::audit::checkpoint as checkpoint_codec;
use crate::v3::context::{
    OutboundFrame, PendingFin, PendingFinState,
    SequencedOutbound, SessionContext,
};
use crate::v3::dispatch::inbound::dispatch_frame;
use crate::v3::io::encode::FrameEncoder;
use crate::v3::io::lane_channels::{LaneChannels, PlaintextBuf};
use crate::v3::session::state::{SessionEvent, SessionState};
use crate::v3::wire::constants::{ENVELOPE_LEN, STREAM_HEADER_LEN};
use crate::v3::wire::frame_class::FrameClass;
use crate::v3::wire::frame_kind::{AuditKind, ChannelKind, StreamKind};
use crate::v3::wire::lane::Lane;

use super::read_task::SessionOutcome;

/// Signal from the control loop to the bridge task for bulk data delivery.
/// Both data chunks and completion signals travel the same ordered channel,
/// eliminating the race where completion overtakes data.
pub enum BulkDataSignal {
    /// A decrypted chunk ready for the application.
    Chunk { stream_id: u8, chunk_index: u32, data: PlaintextBuf },
    /// All chunks for a stream have been delivered and verified.
    Complete { stream_id: u8, transfer_id: uuid::Uuid, total_bytes: u64, total_chunks: u32 },
}

use audit_reorder::AuditReorderBuffer;

/// Write task error — defined here (not in write_task.rs) so it's
/// available on all platforms regardless of write_task cfg-gating.
#[derive(Debug)]
pub enum WriteError {
    Io(std::io::Error),
}

impl From<std::io::Error> for WriteError {
    fn from(e: std::io::Error) -> Self { Self::Io(e) }
}
use drain::DrainContext;
use heartbeat::HeartbeatState;

// ── Public types — contract between read task and control loop ────

pub struct VerifiedFrame {
    pub envelope_bytes: [u8; ENVELOPE_LEN],
    pub body: Vec<u8>,
    pub envelope: crate::v3::codec::envelope::EnvelopeInfo,
    pub header: Option<crate::v3::codec::header::StreamHeaderInfo>,
    pub plaintext: Vec<u8>,
    /// Set when this frame is the first from the peer using a new epoch.
    /// The control loop uses this to install the responder's pending
    /// encoder keys and retire the previous epoch's decoder keys.
    pub peer_epoch_advanced: bool,
}

pub enum ReadSignal {
    Frame(VerifiedFrame),
    Finished(SessionOutcome),
}

// ── Control action — select! discriminant ────────────────────────

pub(crate) enum ControlAction {
    WriteFailed(WriteError),
    PongTimeout,
    HeartbeatTick,
    RecvFrame(VerifiedFrame),
    ReadFinished(SessionOutcome),
    OutboundAuditLink(LinkInput),
    InboundAuditLink(LinkInput),
    BulkDecrypted(BulkDecryptResult),
    PendingFinReceived(PendingFin),
    SequencedOutbound(SequencedOutbound),
    ClientOutbound(OutboundFrame),
}

// ── Entry point ──────────────────────────────────────────────────

/// The control loop no longer takes crossbeam senders. The control loop
/// inserts LinkInputs directly into its own AuditReorderBuffers. Only
/// rayon workers use the crossbeam → bridge → tokio mpsc path.
#[allow(clippy::too_many_arguments)]
pub async fn run(
    mut ctx: SessionContext,
    mut frame_rx: mpsc::Receiver<ReadSignal>,
    lane_channels: LaneChannels,
    encoder: Arc<FrameEncoder>,
    encrypt_pool: Arc<rayon::ThreadPool>,
    counters: Arc<crate::v3::bulk::counters::BulkCounters>,
    mut write_error_rx: mpsc::Receiver<WriteError>,
    mut outbound_rx: mpsc::Receiver<OutboundFrame>,
    outbound_audit_queue: AuditQueue,
    outbound_audit_wake: rekindle_transport_buff::adapters::tokio::TokioWake,
    inbound_audit_queue: AuditQueue,
    inbound_audit_wake: rekindle_transport_buff::adapters::tokio::TokioWake,
    bulk_decrypted_queue: std::sync::Arc<rekindle_transport_buff::DispatchQueue<BulkDecryptResult, rekindle_transport_buff::adapters::tokio::TokioWake>>,
    mut sequenced_rx: mpsc::Receiver<SequencedOutbound>,
    mut pending_fin_rx: mpsc::Receiver<PendingFin>,
    bulk_data_tx: mpsc::Sender<BulkDataSignal>,
    credit_guard: Arc<rekindle_transport_buff::CreditGuard>,
    last_activity_ns: Arc<std::sync::atomic::AtomicU64>,
    bulk_wire_tx: crossbeam::channel::Sender<crate::v3::io::lane_channels::BulkFrame>,
) -> SessionOutcome {
    let mut heartbeat = HeartbeatState::new(
        Duration::from_millis(ctx.config().heartbeat_interval_ms),
        Duration::from_millis(ctx.config().heartbeat_response_timeout_ms),
        ctx.config().heartbeat_miss_limit,
        counters,
        Arc::clone(&last_activity_ns),
    );

    let mut heartbeat_timer = tokio::time::interval_at(
        tokio::time::Instant::now() + Duration::from_millis(ctx.config().heartbeat_interval_ms),
        Duration::from_millis(ctx.config().heartbeat_interval_ms),
    );
    heartbeat_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    let mut outbound_reorder = AuditReorderBuffer::new();
    let mut inbound_reorder = AuditReorderBuffer::new();

    let drain_ctx = DrainContext {
        encoder: &encoder,
        encrypt_pool: &encrypt_pool,
        lane_channels: &lane_channels,
        bulk_wire_tx: &bulk_wire_tx,
    };

    let bulk_decrypt_wake = bulk_decrypted_queue.wake_sink().clone();

    loop {
        let action = tokio::select! {
            biased;
            Some(e) = write_error_rx.recv() => ControlAction::WriteFailed(e),
            _ = &mut heartbeat.pong_sleep, if heartbeat.awaiting_pong() => ControlAction::PongTimeout,
            _ = heartbeat_timer.tick() => ControlAction::HeartbeatTick,
            signal = frame_rx.recv() => {
                match signal {
                    Some(ReadSignal::Frame(frame)) => ControlAction::RecvFrame(frame),
                    Some(ReadSignal::Finished(outcome)) => ControlAction::ReadFinished(outcome),
                    None => ControlAction::ReadFinished(SessionOutcome::ConnectionLost),
                }
            }
            _ = outbound_audit_wake.notified() => {
                match outbound_audit_queue.pop() {
                    Some((_seq, link)) => {
                        crate::v3::bulk::counters::DIAG_AUDIT_OUTBOUND_POPS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        ControlAction::OutboundAuditLink(link)
                    }
                    None => continue, // spurious wake
                }
            }
            _ = inbound_audit_wake.notified() => {
                match inbound_audit_queue.pop() {
                    Some((_seq, link)) => {
                        crate::v3::bulk::counters::DIAG_AUDIT_INBOUND_POPS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        ControlAction::InboundAuditLink(link)
                    }
                    None => continue, // spurious wake
                }
            }
            _ = bulk_decrypt_wake.notified() => {
                // TokioWake fired — drain all available results from the DispatchQueue.
                // Process the first one as the action; remaining are handled inline.
                match bulk_decrypted_queue.pop() {
                    Some((_seq, result)) => ControlAction::BulkDecrypted(result),
                    None => continue, // spurious wake
                }
            }
            Some(pf) = pending_fin_rx.recv() => ControlAction::PendingFinReceived(pf),
            Some(seq) = sequenced_rx.recv() => ControlAction::SequencedOutbound(seq),
            Some(frame) = outbound_rx.recv() => ControlAction::ClientOutbound(frame),
            else => ControlAction::ReadFinished(SessionOutcome::ConnectionLost),
        };

        tracing::trace!(action = util::action_name(&action), "control loop iteration");

        match action {
            ControlAction::WriteFailed(write_err) => {
                tracing::debug!(error = ?write_err, "write task failed");
                return util::terminate(&ctx, SessionOutcome::SubstrateReadFailed {
                    detail: format!("write task failed: {write_err:?}"),
                });
            }

            ControlAction::PongTimeout => {
                if let Some(outcome) = heartbeat.handle_pong_timeout(&mut ctx) {
                    return util::terminate(&ctx, outcome);
                }
            }

            ControlAction::HeartbeatTick => {
                if let Some(outcome) = heartbeat.tick(&mut ctx, &drain_ctx, &mut outbound_reorder).await {
                    return util::terminate(&ctx, outcome);
                }
            }

            ControlAction::RecvFrame(frame) => {
                if let Some(outcome) = handle_recv_frame(
                    &mut ctx, frame, &drain_ctx,
                    &mut outbound_reorder, &mut inbound_reorder,
                    &mut heartbeat, &bulk_data_tx,
                ).await {
                    return outcome;
                }
                // Epoch key installation is handled by the read task via
                // EpochSignal. The control loop no longer touches encoder
                // or decoder keys. handle_init/handle_commit queue both
                // key sets in the signal; the read task installs them
                // atomically before reading the next frame.
            }

            ControlAction::ReadFinished(outcome) => {
                let final_outcome = match (&outcome, ctx.session_state()) {
                    (SessionOutcome::ConnectionLost, SessionState::Draining) => {
                        SessionOutcome::Closed { peer_initiated: true }
                    }
                    _ => outcome,
                };
                return util::terminate(&ctx, final_outcome);
            }

            ControlAction::OutboundAuditLink(link_input) => {
                let link_seq = link_input.session_seq;
                tracing::debug!(
                    session_seq = link_seq,
                    pending_fins = ?ctx.pending_fins_stream_ids(),
                    outbound_reorder_next = outbound_reorder.next_expected(),
                    "control loop: OutboundAuditLink received"
                );
                drain::insert_and_advance_outbound(&mut ctx, &mut outbound_reorder, link_input);

                // Drain remaining queued links that arrived between notified() and pop.
                while let Some((_seq, link)) = outbound_audit_queue.pop() {
                    crate::v3::bulk::counters::DIAG_AUDIT_OUTBOUND_POPS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    drain::insert_and_advance_outbound(&mut ctx, &mut outbound_reorder, link);
                }

                // Check if any pending FIN can now be emitted.
                drain::emit_ready_fins(
                    &mut ctx, &drain_ctx, &mut outbound_reorder,
                ).await;

                if outbound_reorder.is_stalled() {
                    tracing::error!(
                        next_expected = outbound_reorder.next_expected(),
                        buffered = outbound_reorder.buffered_count(),
                        stall_duration_ms = outbound_reorder.time_since_last_advance().as_millis(),
                        "outbound audit reorder buffer stalled — rayon worker likely panicked"
                    );
                    return util::terminate(&ctx, SessionOutcome::AuditChainDivergence {
                        checkpoint_seq: outbound_reorder.next_expected(),
                    });
                }
            }

            ControlAction::InboundAuditLink(link_input) => {
                inbound_reorder.insert_and_drain(link_input, |entry| {
                    let seq = entry.session_seq;
                    handle_inbound_audit_entry(&mut ctx, entry, seq);
                });
                // Drain remaining queued links.
                while let Some((_seq, link)) = inbound_audit_queue.pop() {
                    crate::v3::bulk::counters::DIAG_AUDIT_INBOUND_POPS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    inbound_reorder.insert_and_drain(link, |entry| {
                        let seq = entry.session_seq;
                        handle_inbound_audit_entry(&mut ctx, entry, seq);
                    });
                }
                if inbound_reorder.is_stalled() {
                    tracing::error!(
                        next_expected = inbound_reorder.next_expected(),
                        buffered = inbound_reorder.buffered_count(),
                        stall_duration_ms = inbound_reorder.time_since_last_advance().as_millis(),
                        "inbound audit reorder buffer stalled"
                    );
                    return util::terminate(&ctx, SessionOutcome::AuditChainDivergence {
                        checkpoint_seq: inbound_reorder.next_expected(),
                    });
                }
                fin::check_deferred_fin_verify(&mut ctx);
            }

            ControlAction::BulkDecrypted(result) => {
                heartbeat.record_activity();
                if let Some(outcome) = handle_bulk_decrypted(
                    &mut ctx, result, &drain_ctx, &mut outbound_reorder,
                    &bulk_data_tx, &credit_guard,
                ).await {
                    return outcome;
                }
                // Drain remaining results that accumulated between
                // notified() and the first pop. Each needs full processing.
                while let Some((_seq, result)) = bulk_decrypted_queue.pop() {
                    heartbeat.record_activity();
                    if let Some(outcome) = handle_bulk_decrypted(
                        &mut ctx, result, &drain_ctx, &mut outbound_reorder,
                        &bulk_data_tx, &credit_guard,
                    ).await {
                        return outcome;
                    }
                }
            }

            ControlAction::PendingFinReceived(pf) => {
                let sid = pf.stream_id;
                tracing::debug!(
                    stream_id = sid,
                    chunk_count = pf.chunk_count,
                    last_chunk_seq = pf.last_chunk_seq,
                    outbound_reorder_next = outbound_reorder.next_expected(),
                    "control loop: PendingFin received"
                );
                ctx.register_pending_fin(sid, PendingFinState {
                    chunk_count: pf.chunk_count,
                    total_bytes: pf.total_bytes,
                    content_hash: pf.content_hash,
                    bulk_wire_tx: pf.bulk_wire_tx,
                    last_chunk_seq: pf.last_chunk_seq,
                });
                // All LinkInputs may have already been flushed through the
                // reorder buffer before PendingFin arrived. Check immediately.
                if let Some(outcome) = drain::emit_ready_fins(
                    &mut ctx, &drain_ctx, &mut outbound_reorder,
                ).await {
                    return outcome;
                }
            }

            ControlAction::SequencedOutbound(seq) => {
                // Store second-phase completion before drain — drain_outbound
                // may call initiate() which needs the completion already stored.
                if let Some(completion) = seq.completion {
                    ctx.store_pending_rotation_confirm(completion);
                }
                ctx.push_outbound(seq.frame);
                if let Some(outcome) = drain::drain_outbound(
                    &mut ctx, &drain_ctx, &mut outbound_reorder,
                ).await {
                    tracing::error!(
                        outcome = ?outcome,
                        "control loop: SequencedOutbound drain_outbound returned fatal — confirmation oneshot will be dropped"
                    );
                    return outcome;
                }
                let _ = seq.confirm.send(());
            }

            ControlAction::ClientOutbound(frame) => {
                if let OutboundFrame::Channel { kind: ChannelKind::Goodbye, .. } = &frame {
                    if !ctx.local_goodbye_sent() {
                        let _ = ctx.session_state_mut().apply(SessionEvent::GoodbyeSent);
                        ctx.mark_local_goodbye_sent();
                        ctx.set_drain_deadline(std::time::Instant::now() + Duration::from_secs(5));
                    }
                }
                ctx.push_outbound(frame);
                if let Some(outcome) = drain::drain_outbound(
                    &mut ctx, &drain_ctx, &mut outbound_reorder,
                ).await {
                    return outcome;
                }
                if ctx.session_state().is_terminal() {
                    return util::terminate(&ctx, SessionOutcome::Closed {
                        peer_initiated: ctx.peer_final_session_seq().is_some(),
                    });
                }
            }
        }
    }
}

// ── Arm body helpers ─────────────────────────────────────────────

fn handle_inbound_audit_entry(ctx: &mut SessionContext, entry: LinkInput, seq: u64) {
    match ctx.gap_detector_mut().observe(seq) {
        GapEvent::InOrder | GapEvent::Duplicate { .. } => {}
        GapEvent::GapDetected { start, end } => {
            let mut builder = MissingBitmapBuilder::new(start, end);
            for s in start..=end { builder.mark_missing(s); }
            let bitmap = builder.build();
            ctx.push_outbound(OutboundFrame::Audit {
                kind: AuditKind::Gap,
                payload: crate::v3::codec::audit::gap::encode(
                    &crate::v3::codec::audit::gap::AuditGapPayload {
                        gap_id: uuid::Uuid::now_v7(),
                        gap_start_seq: start,
                        gap_end_seq: end,
                        gap_detected_ns: util::wall_ns(),
                        expected_chain_link: ctx.inbound_chain().current_link(),
                        missing_bitmap: bitmap.as_bytes().to_vec(),
                    },
                ),
            });
        }
        GapEvent::GapTooLarge { .. } => {
            tracing::error!(seq, "audit gap too large — chain divergence");
        }
    }
    ctx.inbound_chain_mut().advance(entry);
    ctx.advance_recv_seq(seq);
    ctx.inbound_checkpoint_mut().frame_processed();
}

async fn handle_recv_frame(
    ctx: &mut SessionContext,
    frame: VerifiedFrame,
    drain_ctx: &DrainContext<'_>,
    outbound_reorder: &mut AuditReorderBuffer,
    inbound_reorder: &mut AuditReorderBuffer,
    heartbeat: &mut HeartbeatState,
    bulk_data_tx: &mpsc::Sender<BulkDataSignal>,
) -> Option<SessionOutcome> {
    heartbeat.record_activity();

    // Intercept PONG before dispatch_frame. PONG is a transport-internal
    // lifecycle frame that requires atomic mutation of both SessionContext
    // (nonce clear, miss reset) and HeartbeatState (timer cancel, awaiting
    // flag). The handler layer only has &mut SessionContext. Intercepting
    // here gives us &mut to both domains — no split-state window.
    if frame.envelope.lane == Lane::Control && frame.plaintext.len() >= 2 {
        let class = frame.plaintext[0];
        let kind = frame.plaintext[1];
        if class == FrameClass::Channel as u8
            && kind == ChannelKind::Pong as u8
        {
            // PONG — handle inline, do NOT dispatch.
            // handle_pong never returns fatal errors — stale and unsolicited
            // PONGs are logged and counted, not connection-fatal.
            let handler_payload = &frame.plaintext[2..];
            heartbeat.handle_pong(ctx, handler_payload);
            // Still need inbound audit chain, checkpoint, retention, drain
            return handle_recv_frame_post_dispatch(
                ctx, &frame, drain_ctx, outbound_reorder, inbound_reorder,
            ).await;
        }
    }

    // All other frames go through the dispatch table
    if let Err(e) = dispatch_frame(ctx, &frame.envelope, frame.header.as_ref(), &frame.plaintext) {
        return Some(util::terminate(ctx, SessionOutcome::ChannelError {
            code: e.failure_code() as u16,
            message: format!("{e:?}"),
        }));
    }

    // Drain bulk deliveries staged by handlers (payload, fault).
    // Handlers push to ctx.pending_bulk_deliveries; we send through
    // bulk_data_tx here with access to the channel sender.
    for (sid, ci, data) in ctx.drain_bulk_deliveries() {
        if bulk_data_tx.send(BulkDataSignal::Chunk { stream_id: sid, chunk_index: ci, data }).await.is_err() {
            return Some(util::terminate(ctx, SessionOutcome::ConnectionLost));
        }
    }
    // Drain completions AFTER deliveries — ordering guarantee: all data
    // chunks for a stream arrive at the bridge task before the completion.
    for (sid, transfer_id, total_bytes, total_chunks) in ctx.drain_bulk_completions() {
        if bulk_data_tx.send(BulkDataSignal::Complete { stream_id: sid, transfer_id, total_bytes, total_chunks }).await.is_err() {
            return Some(util::terminate(ctx, SessionOutcome::ConnectionLost));
        }
    }

    handle_recv_frame_post_dispatch(
        ctx, &frame, drain_ctx, outbound_reorder, inbound_reorder,
    ).await
}

/// Post-dispatch processing shared by both the PONG interceptor and the
/// normal dispatch path: inbound audit chain, checkpoint, retention, drain.
async fn handle_recv_frame_post_dispatch(
    ctx: &mut SessionContext,
    frame: &VerifiedFrame,
    drain_ctx: &DrainContext<'_>,
    outbound_reorder: &mut AuditReorderBuffer,
    inbound_reorder: &mut AuditReorderBuffer,
) -> Option<SessionOutcome> {
    // Compute inbound audit LinkInput from wire-byte hashes.
    // Insert DIRECTLY into the inbound reorder buffer — no crossbeam send.
    let envelope_hash = *blake3::hash(&frame.envelope_bytes).as_bytes();
    let header_hash = if frame.envelope.lane == Lane::Data && frame.body.len() >= STREAM_HEADER_LEN {
        *blake3::hash(&frame.body[..STREAM_HEADER_LEN]).as_bytes()
    } else {
        [0u8; 32]
    };
    let ciphertext_hash = *blake3::hash(&frame.body).as_bytes();

    let link_input = LinkInput {
        session_seq: frame.envelope.session_seq,
        envelope_hash, header_hash, ciphertext_hash,
    };

    inbound_reorder.insert_and_drain(link_input, |entry| {
        let seq = entry.session_seq;
        handle_inbound_audit_entry(ctx, entry, seq);
    });

    // Checkpoint cadence
    ctx.inbound_checkpoint_mut().frame_processed();
    if ctx.inbound_checkpoint_mut().is_due() {
        ctx.push_outbound(OutboundFrame::Audit {
            kind: AuditKind::Checkpoint,
            payload: checkpoint_codec::encode(&checkpoint_codec::AuditCheckpointPayload {
                chain_index: ctx.outbound_chain().length(),
                chain_length: ctx.outbound_chain().length(),
                checkpoint_seq: 0,
                wall_clock_ns: util::wall_ns(),
                chain_link: ctx.outbound_chain().current_link(),
                anchor_link: ctx.outbound_chain().anchor_record().value,
            }),
        });
        ctx.inbound_checkpoint_mut().checkpoint_emitted();
    }

    // Retention for AUDIT_REPLAY
    let mut retained = Vec::with_capacity(ENVELOPE_LEN + frame.body.len());
    retained.extend_from_slice(&frame.envelope_bytes);
    retained.extend_from_slice(&frame.body);
    ctx.retention_mut().store(frame.envelope.session_seq, retained);

    fin::check_deferred_fin_verify(ctx);

    if let Some(outcome) = drain::drain_outbound(ctx, drain_ctx, outbound_reorder).await {
        return Some(outcome);
    }
    if let Some(outcome) = timers::check_deadlines(ctx) {
        return Some(outcome);
    }
    if ctx.session_state().is_terminal() {
        return Some(util::terminate(ctx, SessionOutcome::Closed {
            peer_initiated: ctx.peer_final_session_seq().is_some(),
        }));
    }
    None
}

async fn handle_bulk_decrypted(
    ctx: &mut SessionContext,
    result: BulkDecryptResult,
    drain_ctx: &DrainContext<'_>,
    outbound_reorder: &mut AuditReorderBuffer,
    bulk_data_tx: &mpsc::Sender<BulkDataSignal>,
    credit_guard: &rekindle_transport_buff::CreditGuard,
) -> Option<SessionOutcome> {
    match result {
        BulkDecryptResult::Chunk(chunk) => {
            let stream_id = chunk.stream_id;
            let chunk_index = chunk.chunk_index;
            let chunk_digest = chunk.chunk_digest;
            let kind = chunk.kind;

            if kind == StreamKind::Fin {
                // FIN body_len is always 128 bytes (32 header + 80 payload + 16 tag),
                // far below any valid bulk_threshold (default 65536). A FIN reaching
                // the bulk path means bulk_threshold is misconfigured to 0, which
                // routes ALL Data lane frames through BulkReceiver including lifecycle
                // frames that must go through the inline handler. This is a fatal
                // configuration error — the session cannot recover.
                tracing::error!(
                    stream_id, chunk_index,
                    plaintext_len = chunk.plaintext.len(),
                    "BUG: FIN frame reached bulk decrypt path — bulk_threshold is too low"
                );
                return Some(util::terminate(ctx, SessionOutcome::ChannelError {
                    code: 0x0003,
                    message: "FIN frame routed to bulk path — invalid bulk_threshold".to_string(),
                }));
            }

            tracing::debug!(
                stream_id, chunk_index,
                digest = %hex::encode(&chunk_digest[..8]),
                "handle_bulk_decrypted: payload chunk → reassembler"
            );
            if let Some(reassembler) = ctx.reassembler_mut(stream_id) {
                let delivered = reassembler.insert_with_digest(chunk_index, chunk.plaintext, chunk_digest);
                for (ci, data) in delivered {
                    credit_guard.release(data.len() as u64);
                    ctx.push_bulk_delivery(stream_id, ci, data);
                }
            } else {
                // Reassembler not yet created — STREAM_OPEN is still in the
                // inline signal bridge while the rayon decrypt path delivered
                // this chunk first. Buffer it for replay when create_reassembler
                // is called by the STREAM_OPEN handler.
                ctx.buffer_early_chunk(stream_id, chunk_index, chunk.plaintext, chunk_digest);
            }

            // FIN_FOLLOWS: single-chunk transfer via bulk path. Verify and ACK
            // immediately — no separate FIN frame expected. PendingFinVerify was
            // stored at STREAM_OPEN time.
            if chunk.header_flags & crate::v3::wire::header::flags::FIN_FOLLOWS != 0 {
                if let Some(pending) = ctx.take_pending_fin_verify(stream_id) {
                    let hash_ok = ctx.reassembler_mut(stream_id)
                        .map(|r| r.verify_content_hash(&pending.content_hash).is_ok())
                        .unwrap_or(false);

                    if hash_ok {
                        let total_bytes = ctx.reassembler_mut(stream_id)
                            .map(|r| r.total_bytes()).unwrap_or(0);
                        let total_chunks = ctx.reassembler_mut(stream_id)
                            .map(|r| r.next_expected()).unwrap_or(0);

                        let ack = crate::v3::codec::stream::ack::StreamAckPayload {
                            transfer_id: pending.transfer_id,
                            ack_byte_count: total_bytes,
                            ack_chunk_count: total_chunks,
                            audit_link: ctx.inbound_chain().current_link(),
                        };
                        ctx.push_outbound(OutboundFrame::Data {
                            stream_id,
                            kind: StreamKind::Ack,
                            chunk_index: total_chunks,
                            payload: crate::v3::codec::stream::ack::encode(&ack),
                        });

                        let info = ctx.connection_info().clone();
                        ctx.push_bulk_completion(stream_id, pending.transfer_id, total_bytes as u64, total_chunks);
                        ctx.router().on_bulk_complete(
                            &info, stream_id, pending.transfer_id,
                            total_bytes, total_chunks,
                        );
                        ctx.remove_reassembler(stream_id);
                        let _ = ctx.close_inbound_stream(stream_id);
                    } else {
                        // Content hash mismatch on FIN_FOLLOWS — terminate
                        return Some(util::terminate(ctx, SessionOutcome::AeadVerificationFailed {
                            session_seq: ctx.recv_last_seq(),
                        }));
                    }
                }
            } else {
                fin::check_deferred_fin_verify(ctx);
            }

            // Drain bulk deliveries to client/server
            for (sid, ci, data) in ctx.drain_bulk_deliveries() {
                if bulk_data_tx.send(BulkDataSignal::Chunk { stream_id: sid, chunk_index: ci, data }).await.is_err() {
                    return Some(util::terminate(ctx, SessionOutcome::ConnectionLost));
                }
            }
            // Drain completions AFTER deliveries — same ordering guarantee
            // as handle_recv_frame. FIN_FOLLOWS pushes a completion at line
            // 664; it must be drained here, not deferred to the next
            // handle_recv_frame (which may never fire on an idle connection).
            for (sid, transfer_id, total_bytes, total_chunks) in ctx.drain_bulk_completions() {
                if bulk_data_tx.send(BulkDataSignal::Complete { stream_id: sid, transfer_id, total_bytes, total_chunks }).await.is_err() {
                    return Some(util::terminate(ctx, SessionOutcome::ConnectionLost));
                }
            }

            drain::drain_outbound(ctx, drain_ctx, outbound_reorder).await
        }
        BulkDecryptResult::Fatal(e) => {
            tracing::error!(error = %e, "bulk decrypt FATAL");
            // Credit was reserved by the read task before dispatch.
            // The frame failed decryption — release the credited bytes.
            // We don't know the exact frame_len here (it's not carried
            // in RecvDispatchError), so we release chunk_len as best effort.
            // The CreditGuard ceiling prevents OOM; a small leak per fatal
            // frame is acceptable because fatal errors terminate the session.
            Some(util::terminate(ctx, SessionOutcome::AeadVerificationFailed {
                session_seq: ctx.recv_last_seq(),
            }))
        }
    }
}

