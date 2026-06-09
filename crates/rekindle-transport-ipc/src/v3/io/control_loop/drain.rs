//! Outbound frame encoding pipeline — inline for small frames, rayon for large.
//!
//! Frames with payload < INLINE_ENCODE_THRESHOLD are encoded synchronously on
//! the control loop thread. Avoids rayon spawn + oneshot overhead (~5 µs) for
//! frames whose AEAD cost is < 500 ns.
//!
//! Frames with payload >= INLINE_ENCODE_THRESHOLD are dispatched to the rayon
//! pool where the AEAD cost dominates the spawn overhead.
//!
//! The control loop inserts LinkInputs DIRECTLY into the AuditReorderBuffer —
//! no channel, no bridge thread, no blocking. Only rayon workers use the
//! crossbeam → bridge → tokio mpsc path (they are OS threads that cannot
//! call tokio async send).

use std::sync::Arc;

use crate::v3::audit::chain::LinkInput;
use crate::v3::context::{OutboundFrame, SessionContext};
use crate::v3::io::encode::FrameEncoder;
use crate::v3::io::lane_channels::LaneChannels;
use crate::v3::io::read_task::SessionOutcome;
use crate::v3::stream::registry::Direction;
use crate::v3::stream::state::StreamEvent;
use crate::v3::wire::frame_kind::StreamKind;

use super::audit_reorder::AuditReorderBuffer;
use super::fin;
use super::util;

/// Inline encode threshold in bytes. Frames with payload below this
/// are encoded synchronously on the control loop thread.
/// Set to 8192 to account for AEGIS-128X2 where AEAD at 4 KiB is only
/// ~400 ns — below the rayon spawn cost of ~500 ns.
const INLINE_ENCODE_THRESHOLD: usize = 8192;


/// Bundled immutable references needed by drain_outbound. Constructed
/// once in run() and passed by reference.
pub(super) struct DrainContext<'a> {
    pub encoder: &'a Arc<FrameEncoder>,
    pub encrypt_pool: &'a Arc<rayon::ThreadPool>,
    pub lane_channels: &'a LaneChannels,
    /// Bulk wire channel sender — used to route STREAM_OPEN through the
    /// same channel as STREAM_PAYLOAD so the write task coalesces them
    /// into a single writev syscall.
    pub bulk_wire_tx: &'a crossbeam::channel::Sender<crate::v3::io::lane_channels::BulkFrame>,
}

/// Drain all queued outbound frames from SessionContext. Encodes each
/// frame, inserts the LinkInput directly into the outbound reorder buffer,
/// and routes wire bytes to the appropriate lane channel. After all frames
/// are encoded, checks if any pending FIN can now be emitted.
///
/// Returns Some(SessionOutcome) on fatal error (channel closed, rayon panic).
pub(super) async fn drain_outbound(
    ctx: &mut SessionContext,
    drain_ctx: &DrainContext<'_>,
    outbound_reorder: &mut AuditReorderBuffer,
) -> Option<SessionOutcome> {
    let frames = ctx.drain_outbound();
    if frames.is_empty() {
        return None;
    }
    tracing::debug!(count = frames.len(), "drain_outbound: encoding frames");

    for mut outbound in frames {
        // Client-initiated rotation: empty-payload RotateInit is a trigger.
        // Call initiate() to produce real payload, transition state, set deadline.
        if let OutboundFrame::Channel { kind: crate::v3::wire::frame_kind::ChannelKind::RotateInit, ref mut payload } = &mut outbound {
            if payload.is_empty() {
                tracing::info!(
                    state = ?ctx.session_state(),
                    "drain_outbound: empty-payload RotateInit detected — calling initiate()"
                );
                let transcript_anchor = ctx.outbound_chain().current_link();
                match ctx.rotation_mut().initiate(5000, transcript_anchor) {
                    Ok(init_payload) => {
                        *payload = crate::v3::codec::channel::rotate::encode_init(&init_payload);
                        let _ = ctx.session_state_mut().apply(
                            crate::v3::session::state::SessionEvent::RotateInitSent,
                        );
                        ctx.set_rotation_deadline(
                            std::time::Instant::now() + std::time::Duration::from_millis(5000),
                        );
                        tracing::info!(
                            state = ?ctx.session_state(),
                            "drain_outbound: RotateInit encoded, state transitioned to Rotating"
                        );
                    }
                    Err(e) => {
                        tracing::error!(?e, "rotation initiate failed");
                        continue;
                    }
                }
            }
        }

        // Stream registry transitions must happen synchronously before encoding
        if let OutboundFrame::Data { stream_id, kind, ref mut payload, .. } = &mut outbound {
            match kind {
                StreamKind::Open => {
                    if let Err(e) = ctx.stream_registry_mut().open(*stream_id, Direction::Outbound) {
                        return Some(util::terminate(ctx, SessionOutcome::ChannelError {
                            code: 0x0005,
                            message: format!("stream_id {} open failed: {e}", stream_id),
                        }));
                    }
                    ctx.create_reassembler(*stream_id);
                }
                StreamKind::Fin => {
                    let _ = ctx.stream_registry_mut().transition(*stream_id, Direction::Outbound, StreamEvent::FinSent);
                    if payload.len() >= 80 {
                        let audit_link = ctx.outbound_chain().current_link();
                        payload[48..80].copy_from_slice(&audit_link);
                    }
                }
                StreamKind::Cancel => {
                    let _ = ctx.stream_registry_mut().transition(*stream_id, Direction::Outbound, StreamEvent::CancelSent);
                }
                StreamKind::Reset => {
                    let _ = ctx.stream_registry_mut().transition(*stream_id, Direction::Outbound, StreamEvent::ResetSent);
                }
                _ => {}
            }
        }

        // When encoding a DATAGRAM_REPLY, resolve the pending request
        // slot registered by the inbound request handler. This frees
        // the slot when the APPLICATION sends its reply — bounding
        // concurrent in-flight application work at max_pending_requests.
        if let OutboundFrame::Datagram {
            kind: crate::v3::wire::frame_kind::DatagramKind::Reply, payload
        } = &outbound {
            if payload.len() >= 32 {
                if let Ok(bytes) = <[u8; 16]>::try_from(&payload[16..32]) {
                    let correlation_id = uuid::Uuid::from_bytes(bytes);
                    ctx.resolve_pending_request(&correlation_id);
                }
            }
        }

        let payload_len = match &outbound {
            OutboundFrame::Channel { payload, .. }
            | OutboundFrame::Datagram { payload, .. }
            | OutboundFrame::Data { payload, .. }
            | OutboundFrame::Audit { payload, .. }
            | OutboundFrame::Handoff { payload, .. } => payload.len(),
        };

        if payload_len < INLINE_ENCODE_THRESHOLD {
            // Inline encode — no rayon overhead for small frames.
            let ewa = drain_ctx.encoder.encode_with_audit(&outbound);
            // Direct insert into outbound reorder buffer — no channel, no blocking.
            insert_and_advance_outbound(ctx, outbound_reorder, ewa.link_input);

            // Route STREAM_OPEN through the bulk channel so the write task
            // coalesces it with the immediately-following STREAM_PAYLOAD
            // into a single writev syscall. All other frames go to their
            // normal lane channel.
            let is_stream_open = matches!(
                &outbound,
                OutboundFrame::Data { kind: StreamKind::Open, .. }
            );
            if is_stream_open {
                use crate::v3::io::lane_channels::BulkFrame;
                if drain_ctx.bulk_wire_tx.try_send(BulkFrame::Plain(ewa.encoded.into_wire_bytes())).is_err() {
                    return Some(util::terminate(ctx, SessionOutcome::ConnectionLost));
                }
            } else if drain_ctx.lane_channels
                .send_with_retry(ewa.lane, ewa.encoded.into_wire_bytes()).await.is_err()
            {
                return Some(util::terminate(ctx, SessionOutcome::ConnectionLost));
            }
        } else {
            // Large frame — dispatch to rayon pool.
            let (tx, rx) = tokio::sync::oneshot::channel();
            let enc = Arc::clone(drain_ctx.encoder);
            drain_ctx.encrypt_pool.spawn(move || {
                let ewa = enc.encode_with_audit(&outbound);
                let _ = tx.send(ewa);
            });

            let ewa = match rx.await {
                Ok(ewa) => ewa,
                Err(_) => {
                    tracing::error!("encrypt pool worker panicked during encode");
                    return Some(util::terminate(ctx, SessionOutcome::SubstrateReadFailed {
                        detail: "encrypt pool worker panicked".to_string(),
                    }));
                }
            };

            // Direct insert — the rayon worker returned the LinkInput via
            // oneshot (async, non-blocking). No crossbeam send needed.
            insert_and_advance_outbound(ctx, outbound_reorder, ewa.link_input);
            if drain_ctx.lane_channels
                .send_with_retry(ewa.lane, ewa.encoded.into_wire_bytes()).await.is_err()
            {
                return Some(util::terminate(ctx, SessionOutcome::ConnectionLost));
            }
        }
    }

    // After encoding all frames, check if any pending FIN streams have
    // received all their payload LinkInputs and can now emit STREAM_FIN.
    // This is the inline-encode FIN emission path — for rayon-produced
    // LinkInputs, the OutboundAuditLink arm handles FIN emission.
    emit_ready_fins(ctx, drain_ctx, outbound_reorder).await
}

/// Insert a LinkInput into the outbound reorder buffer and flush any
/// contiguous entries, advancing the outbound audit chain.
///
/// Does NOT check pending FINs — that is the caller's responsibility
/// after all LinkInputs for a batch have been inserted.
pub(super) fn insert_and_advance_outbound(
    ctx: &mut SessionContext,
    reorder: &mut AuditReorderBuffer,
    link_input: LinkInput,
) {
    reorder.insert_and_drain(link_input, |entry| {
        ctx.outbound_chain_mut().advance(entry);
    });
}

/// Check all pending FIN streams. For each stream where all payload
/// LinkInputs have been received, emit STREAM_FIN. This is async
/// because emit_stream_fin dispatches to rayon and awaits the oneshot.
pub(super) async fn emit_ready_fins(
    ctx: &mut SessionContext,
    drain_ctx: &DrainContext<'_>,
    outbound_reorder: &mut AuditReorderBuffer,
) -> Option<SessionOutcome> {
    let reorder_next = outbound_reorder.next_expected();
    let pending_stream_ids: Vec<u8> = ctx.pending_fins_stream_ids();
    for sid in pending_stream_ids {
        if let Some(state) = ctx.pending_fin_mut(sid) {
            tracing::debug!(
                stream_id = sid,
                last_chunk_seq = state.last_chunk_seq,
                reorder_next,
                chunk_count = state.chunk_count,
                ready = reorder_next > state.last_chunk_seq,
                "emit_ready_fins: checking"
            );
        }
        if let Some(completed) = ctx.take_ready_pending_fin(sid, reorder_next) {
            tracing::debug!(
                stream_id = sid,
                chunk_count = completed.chunk_count,
                last_chunk_seq = completed.last_chunk_seq,
                reorder_next,
                "emit_ready_fins: all LinkInputs flushed, emitting STREAM_FIN"
            );
            fin::emit_stream_fin(
                ctx, drain_ctx.encoder, drain_ctx.encrypt_pool,
                outbound_reorder, sid, &completed,
            ).await;
            tracing::debug!(stream_id = sid, "emit_ready_fins: STREAM_FIN emitted");
        }
    }
    None
}
