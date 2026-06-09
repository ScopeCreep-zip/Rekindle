//! CHANNEL_PONG handler — two-generation nonce verification.
//!
//! Custom extension: SCTP §8.3 uses a single nonce with silent discard
//! on mismatch. The v3 model accepts PONGs matching either the current
//! or previous PING nonce generation. Stale PONGs (previous generation)
//! reset the miss count — the peer IS alive, the response was delayed.
//! Unsolicited PONGs (matching neither) are logged and discarded.
//!
//! This is the single source of truth for PONG nonce verification and
//! SessionContext mutation. The control loop's PONG interceptor calls
//! verify_and_apply then additionally cancels HeartbeatState timers.

use crate::v3::codec::channel::ping as codec;
use crate::v3::context::SessionContext;
use crate::v3::handlers::HandlerError;

/// Result of PONG nonce verification.
pub enum PongOutcome {
    /// Nonce matched current generation — full reset.
    /// Carries the echoed sender_epoch_ns from the original PING
    /// for RTT computation: RTT = now_ns - ping_sent_epoch_ns.
    Accepted { ping_sent_epoch_ns: u64 },
    /// Nonce matched previous generation — stale but peer is alive.
    AcceptedStale,
    /// Nonce matched neither — discarded silently.
    Discarded,
}

/// Verify PONG nonce against two-generation state in SessionContext
/// and perform all SessionContext mutations. Returns the outcome
/// so the caller (control loop interceptor) can additionally cancel
/// HeartbeatState timers on Accepted/AcceptedStale.
pub fn verify_and_apply(ctx: &mut SessionContext, payload: &[u8]) -> Result<PongOutcome, HandlerError> {
    let pong = codec::decode(payload).map_err(|e| HandlerError::CodecFailed(format!("{e:?}")))?;

    let received_nonce = pong.ping_nonce;

    // Check current generation (most common case)
    if ctx.last_ping_nonce() == Some(received_nonce) {
        ctx.clear_all_ping_nonces();
        ctx.reset_heartbeat_miss();
        ctx.set_remote_last_seen_our_seq(pong.last_seen_remote_seq);
        ctx.record_pong_received();
        return Ok(PongOutcome::Accepted { ping_sent_epoch_ns: pong.sender_epoch_ns });
    }

    // Check previous generation (stale PONG arrived after timeout)
    if ctx.previous_ping_nonce() == Some(received_nonce) {
        ctx.clear_previous_ping_nonce();
        ctx.reset_heartbeat_miss();
        ctx.set_remote_last_seen_our_seq(pong.last_seen_remote_seq);
        ctx.record_pong_received();
        return Ok(PongOutcome::AcceptedStale);
    }

    // Neither generation matches — discard silently.
    tracing::warn!(
        received_nonce,
        current = ?ctx.last_ping_nonce(),
        previous = ?ctx.previous_ping_nonce(),
        "PONG nonce matches no outstanding PING — discarded"
    );
    Ok(PongOutcome::Discarded)
}

/// dispatch_frame entry point. No-op — PONG is intercepted by the control
/// loop before dispatch_frame is called. This stub exists so the dispatch
/// table has a handler for every (class, kind) pair. verify_and_apply is
/// called directly by the control loop's PONG interceptor which has &mut
/// access to both SessionContext and HeartbeatState.
pub fn handle(_ctx: &mut SessionContext, _payload: &[u8]) -> Result<(), HandlerError> {
    Ok(())
}
