//! CHANNEL_PONG handler — two-generation nonce verification.
//!
//! verify_and_apply is called by the Control lane's PONG interceptor
//! which has access to both ControlState and HeartbeatState. The
//! dispatch_frame path calls the `handle` function which is unreachable —
//! the Control lane intercepts PONGs before dispatch.

use std::time::Instant;

use crate::v4::codec::channel::ping as codec;
use crate::v4::handlers::HandlerError;

/// Result of PONG nonce verification.
pub enum PongOutcome {
    Accepted { ping_sent_epoch_ns: u64 },
    AcceptedStale,
    Discarded,
}

/// Verify PONG nonce against two-generation state in ControlState.
/// Called by the Control lane's PONG interceptor (lane/control.rs).
pub fn verify_and_apply(
    last_ping_nonce: &mut Option<u64>,
    previous_ping_nonce: &mut Option<u64>,
    heartbeat_miss_count: &mut u32,
    remote_last_seen_our_seq: &mut u64,
    last_pong_received: &mut Option<Instant>,
    payload: &[u8],
) -> Result<PongOutcome, HandlerError> {
    let pong = codec::decode(payload)
        .map_err(|e| HandlerError::CodecFailed(format!("{e:?}")))?;

    let received_nonce = pong.ping_nonce;

    if *last_ping_nonce == Some(received_nonce) {
        *last_ping_nonce = None;
        *previous_ping_nonce = None;
        *heartbeat_miss_count = 0;
        *remote_last_seen_our_seq = pong.last_seen_remote_seq;
        *last_pong_received = Some(Instant::now());
        return Ok(PongOutcome::Accepted { ping_sent_epoch_ns: pong.sender_epoch_ns });
    }

    if *previous_ping_nonce == Some(received_nonce) {
        *previous_ping_nonce = None;
        *heartbeat_miss_count = 0;
        *remote_last_seen_our_seq = pong.last_seen_remote_seq;
        *last_pong_received = Some(Instant::now());
        return Ok(PongOutcome::AcceptedStale);
    }

    tracing::warn!(
        received_nonce,
        current = ?last_ping_nonce,
        previous = ?previous_ping_nonce,
        "PONG nonce matches no outstanding PING — discarded"
    );
    Ok(PongOutcome::Discarded)
}

/// dispatch_frame entry point. PONG is intercepted by the Control lane
/// task at control.rs lines 95-121 BEFORE dispatch_control is called.
/// If this function executes, the intercept failed — a code change
/// broke the PONG handling path. Nonce verification would be skipped,
/// allowing unsolicited PONGs to reset heartbeat miss count without
/// validation.
pub fn handle(_payload: &[u8]) -> Result<(), HandlerError> {
    tracing::error!(
        "PONG reached dispatch — Control lane intercept did not fire. \
         This is a bug: PONGs must be handled inline with access to \
         HeartbeatState for nonce verification and timer cancellation."
    );
    Err(HandlerError::CodecFailed(
        "PONG bypassed Control lane intercept — nonce verification skipped".into(),
    ))
}
