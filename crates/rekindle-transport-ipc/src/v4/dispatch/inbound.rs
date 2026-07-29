//! Per-lane dispatch functions — the single wiring point between lane
//! tasks and handlers.
//!
//! Four functions, one per lane. Each takes its lane's state struct by
//! `&mut`, destructures it, and passes exact subsystem references to
//! each handler. A handler cannot access state outside its parameter
//! list — the compiler enforces isolation.
//!
//! Adding a new frame kind: add one match arm in the correct lane's
//! dispatch function. The handler's parameter list documents exactly
//! what subsystem state it needs.

use crate::v4::codec::header::StreamHeaderInfo;
use crate::v4::handlers::{self, HandlerError};
use crate::v4::io::control_loop::lane::state::{
    AuditState, ControlState, DataState, HandoffState,
};
use crate::v4::io::control_loop::shared_state::SessionStateHandle;
use crate::v4::router::{ConnectionInfo, FrameRouter};
use crate::v4::wire::outbound::OutboundFrame;

// ── Handoff lane (0x05) ─────────────────────────────────────────

pub fn dispatch_handoff(
    state: &mut HandoffState,
    router: &dyn FrameRouter,
    info: &ConnectionInfo,
    outbound: &mut Vec<OutboundFrame>,
    kind: u8,
    payload: &[u8],
) -> Result<(), HandlerError> {
    tracing::debug!(
        kind,
        payload_len = payload.len(),
        arena_count = state.arenas.len(),
        "dispatch_handoff: dispatching"
    );
    match kind {
        #[cfg(target_os = "linux")]
        0x01 => handlers::streaming::arena_write::handle(
            &state.arenas, router, info, outbound, payload,
        ),
        #[cfg(target_os = "linux")]
        0x02 => handlers::streaming::slot_release::handle(
            &state.arenas, payload,
        ),
        #[cfg(target_os = "linux")]
        0x03 => handlers::streaming::arena_setup::handle(
            &mut state.arenas, &mut state.pending_arena_fds, &state.sidechannel,
            outbound, payload,
        ),
        #[cfg(target_os = "linux")]
        0x04 => handlers::streaming::arena_ack::handle(
            &mut state.arenas, state.conn_id, payload,
        ),
        0x05 => handlers::streaming::dmabuf_ref::handle(
            router, info, payload,
        ),
        #[cfg(not(target_os = "linux"))]
        0x01..=0x04 => Err(HandlerError::CodecFailed(
            "streaming frames not supported on non-Linux".into(),
        )),
        _ => Err(HandlerError::FrameDisallowedInState),
    }
}

// ── Data lane (0x02) ────────────────────────────────────────────

pub fn dispatch_data(
    state: &mut DataState,
    router: &dyn FrameRouter,
    info: &ConnectionInfo,
    outbound: &mut Vec<OutboundFrame>,
    header: Option<&StreamHeaderInfo>,
    class: u8,
    kind: u8,
    payload: &[u8],
) -> Result<(), HandlerError> {
    // Rerouted Channel frames (credit, backpressure) have class 0x01
    // and no stream header. Handle them first.
    if class == 0x01 {
        return match kind {
            0x09 => handlers::channel::credit::handle(
                &mut state.stream_credits,
                &mut state.lane_credit_bytes,
                payload,
            ),
            0x0A => handlers::channel::backpressure::handle_assert(
                &mut state.backpressure,
                payload,
            ),
            0x0B => { handlers::channel::backpressure::handle_clear(
                &mut state.backpressure,
            ); Ok(()) },
            _ => Err(HandlerError::FrameDisallowedInState),
        };
    }

    // Stream frames (class 0x02) require a stream header.
    let h = header.ok_or(HandlerError::CodecFailed("missing stream header".into()))?;
    match kind {
        0x01 => handlers::stream::open::handle(
            &mut state.stream_registry,
            &mut state.reassemblers,
            &mut state.stream_credits,
            &mut state.pending_fin_verify,
            &mut state.early_bulk_chunks,
            &mut state.pending_bulk_deliveries,
            state.agreed_clearance,
            &state.config,
            h, payload,
        ),
        0x02 => handlers::stream::payload::handle(
            &mut state.reassemblers,
            &mut state.pending_fin_verify,
            &mut state.pending_bulk_deliveries,
            &mut state.pending_bulk_completions,
            &mut state.stream_registry,
            router, info, outbound,
            h, payload,
        ),
        0x03 => handlers::stream::fault::handle(
            &mut state.reassemblers,
            &mut state.pending_bulk_deliveries,
            h, payload,
        ),
        0x04 => handlers::stream::fin::handle(
            &mut state.reassemblers,
            &mut state.stream_registry,
            &mut state.pending_fin_verify,
            &mut state.pending_bulk_completions,
            outbound,
            router, info,
            h, payload,
        ),
        0x05 => handlers::stream::ack::handle(
            &mut state.stream_registry,
            &mut state.reassemblers,
            router, info,
            h, payload,
        ),
        0x06 => handlers::stream::nack::handle(
            &mut state.stream_registry,
            router, info,
            h, payload,
        ),
        0x07 => handlers::stream::reset::handle(
            &mut state.stream_registry,
            &mut state.reassemblers,
            h, payload,
        ),
        0x08 => handlers::stream::cancel::handle(
            &mut state.reassemblers,
            &mut state.stream_registry,
            &mut state.resume_registry,
            state.active_capabilities,
            outbound,
            &state.config,
            h, payload,
        ),
        0x09 => Ok(()), // CancelAck
        0x0A => handlers::stream::resume::handle(
            &mut state.stream_registry,
            &mut state.reassemblers,
            &mut state.stream_credits,
            &mut state.early_bulk_chunks,
            &mut state.pending_bulk_deliveries,
            &state.resume_registry,
            &state.config,
            outbound,
            h, payload,
        ),
        0x0B => Ok(()), // ResumeDeny
        0x0C => handlers::stream::credit::handle(
            &mut state.stream_credits,
            h, payload,
        ),
        0x0D => handlers::stream::reference::handle(
            &state.receiver_cache,
            outbound,
            h, payload,
        ),
        0x0E => {
            let mut retention = state.retention.lock();
            handlers::stream::sack::handle(
                &state.chunk_to_seq,
                &mut retention,
                h, payload,
            )
        }
        _ => Err(HandlerError::FrameDisallowedInState),
    }
}

// ── Control lane (0x01 + 0x03) ──────────────────────────────────

pub fn dispatch_control(
    state: &mut ControlState,
    shared: &SessionStateHandle,
    router: &dyn FrameRouter,
    info: &ConnectionInfo,
    outbound: &mut Vec<OutboundFrame>,
    class: u8,
    kind: u8,
    payload: &[u8],
) -> Result<(), HandlerError> {
    match (class, kind) {
        // ── Channel (0x01) ────────────────────────────────
        (0x01, 0x01) => Err(HandlerError::FrameDisallowedInState), // Hello
        (0x01, 0x02) => Err(HandlerError::FrameDisallowedInState), // HelloAck
        (0x01, 0x03) => handlers::channel::goodbye::handle(
            shared, outbound,
            &mut state.peer_final_session_seq,
            &mut state.local_goodbye_sent,
            &mut state.drain_deadline,
            state.send_seq,
            payload,
        ),
        (0x01, 0x04) => handlers::channel::goodbye_ack::handle(
            shared,
            state.local_goodbye_sent,
            state.peer_final_session_seq,
            payload,
        ),
        (0x01, 0x05) => handlers::channel::ping::handle(
            outbound,
            &mut state.remote_last_seen_our_seq,
            state.audit_links.recv_last_seq.load(std::sync::atomic::Ordering::Acquire),
            payload,
        ),
        (0x01, 0x06) => handlers::channel::pong::handle(payload),
        (0x01, 0x07) => handlers::channel::ack::handle(
            &mut state.pending_requests,
            router, info,
            payload,
        ),
        (0x01, 0x08) => handlers::channel::nack::handle(
            &mut state.pending_requests,
            payload,
        ),
        // Credit and Backpressure are rerouted to Data lane by
        // wire::lane::processing_lane. They never arrive here.
        (0x01, 0x09) | (0x01, 0x0A) | (0x01, 0x0B) => {
            tracing::error!(class, kind, "credit/backpressure reached Control dispatch — reroute failed");
            Err(HandlerError::FrameDisallowedInState)
        }
        (0x01, 0x0C) => handlers::channel::rotate::handle_init(
            shared, outbound,
            &mut state.rotation,
            &mut state.rotation_deadline,
            &mut state.signal_epoch,
            &state.epoch_signal,
            &state.audit_links,
            &state.audit_merge_tx,
            state.role,
            state.agreed_aead,
            payload,
        ),
        (0x01, 0x0D) => handlers::channel::rotate::handle_commit(
            shared, outbound,
            &mut state.rotation,
            &mut state.rotation_deadline,
            &mut state.pending_rotation_confirm,
            &mut state.signal_epoch,
            &state.epoch_signal,
            &state.audit_links,
            &state.audit_merge_tx,
            state.role,
            state.agreed_aead,
            payload,
        ),
        (0x01, 0x0E) => handlers::channel::revoke::handle(
            &mut state.subscriptions,
            &state.revocation_tx,
            payload,
        ),
        (0x01, 0x0F) => handlers::channel::error::handle(shared, payload),
        (0x01, 0x10) => handlers::channel::subscribe::handle(
            &mut state.subscriptions,
            outbound,
            payload,
        ),
        (0x01, 0x11) => Ok(()), // SubscribeAck
        (0x01, 0x12) => Ok(()), // SubscribeDeny
        (0x01, 0x13) => handlers::channel::unsubscribe::handle(
            &mut state.subscriptions,
            outbound,
            payload,
        ),
        (0x01, 0x14) => Ok(()), // UnsubscribeAck
        (0x01, 0x15) => Ok(()), // ConditionsUpdate
        (0x01, 0x16) => Ok(()), // CapabilitiesQuery
        (0x01, 0x17) => Ok(()), // CapabilitiesReply
        (0x01, 0x18) => handlers::channel::quiesce::handle_quiesce(
            shared, outbound,
            &mut state.quiescence_deadline,
            payload,
        ),
        (0x01, 0x19) => Ok(()), // QuiesceAck
        (0x01, 0x1A) => handlers::channel::quiesce::handle_resume(
            shared, outbound,
            &mut state.quiescence_deadline,
            payload,
        ),
        (0x01, 0x1B) => Ok(()), // ResumeAck
        (0x01, 0x1C) => Ok(()), // SidechannelCredit

        // ── Datagram (0x03) ──────────────────────────────
        (0x03, 0x01) => handlers::datagram::request::handle(
            &mut state.pending_requests,
            router, info, outbound,
            payload,
        ),
        (0x03, 0x02) => handlers::datagram::reply::handle(
            &mut state.pending_requests,
            router, info,
            payload,
        ),
        (0x03, 0x03) => handlers::datagram::notify::handle(
            router, info, outbound,
            payload,
        ),
        (0x03, 0x04) => handlers::datagram::publish::handle(
            &state.subscriptions,
            state.agreed_clearance,
            router, info, outbound,
            payload,
        ),
        (0x03, 0x05) => handlers::datagram::reject::handle(
            &mut state.pending_requests,
            router, info,
            payload,
        ),

        _ => Err(HandlerError::FrameDisallowedInState),
    }
}

// ── Audit lane (0x04) ───────────────────────────────────────────

pub fn dispatch_audit(
    state: &mut AuditState,
    outbound: &mut Vec<OutboundFrame>,
    kind: u8,
    payload: &[u8],
) -> Result<(), HandlerError> {
    match kind {
        0x01 => handlers::audit::checkpoint::handle(
            &state.audit_links, outbound, payload,
        ),
        0x02 => handlers::audit::query::handle(
            &state.audit_links, outbound, payload,
        ),
        0x03 => handlers::audit::proof::handle(
            &mut state.verified_proofs,
            outbound, payload,
        ),
        0x04 => {
            let retention = state.retention.lock();
            handlers::audit::gap::handle(
                &retention,
                outbound, payload,
            )
        }
        0x05 => {
            let mut retention = state.retention.lock();
            handlers::audit::replay::handle(
                &mut retention,
                &state.audit_merge_tx,
                payload,
            )
        }
        _ => Err(HandlerError::FrameDisallowedInState),
    }
}
