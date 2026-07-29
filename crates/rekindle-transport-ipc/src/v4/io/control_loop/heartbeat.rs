//! Heartbeat state machine — owns timer and miss-count lifecycle.
//!
//! Two-generation nonce model: ControlState is the SOLE AUTHORITY
//! for nonce state (last_ping_nonce, previous_ping_nonce). HeartbeatState
//! owns ONLY timer state (awaiting_pong, pong_sleep).
//!
//! A PONG is valid if it matches either the current or previous nonce
//! generation in ControlState. Stale PONGs (previous generation) reset
//! the miss count — the peer IS alive, the response was delayed.
//! Unsolicited PONGs (matching neither) are logged and discarded.
//!
//! Pong timeout suppression: if the connection has received ANY frame
//! (bulk data, control, audit) within the pong timeout window, the peer
//! is demonstrably alive. A pong timeout in this case means the peer's
//! control loop was too busy to process our PING — not that the peer is
//! dead. Only miss_limit CONSECUTIVE timeouts with NO activity between
//! them are connection-fatal.

use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use tokio::time::Sleep;

use crate::v4::bulk::counters::BulkCounters;
use crate::v4::handlers::channel::pong::{verify_and_apply, PongOutcome};
use crate::v4::io::control_loop::lane::state::ControlState;

use super::util;

/// All heartbeat-related state for one connection.
/// Nonce state lives exclusively in ControlState — not duplicated here.
///
/// `last_activity_ns` is an `Arc<AtomicU64>` shared with the read task.
/// The read task writes to it on every frame received from the socket.
pub(crate) struct HeartbeatState {
    interval: Duration,
    last_activity_ns: Arc<std::sync::atomic::AtomicU64>,
    awaiting_pong: bool,
    pub pong_sleep: Pin<Box<Sleep>>,
    counters: Arc<BulkCounters>,
}

impl HeartbeatState {
    pub fn new(
        interval: Duration,
        counters: Arc<BulkCounters>,
        last_activity_ns: Arc<std::sync::atomic::AtomicU64>,
    ) -> Self {
        last_activity_ns.store(util::wall_ns(), Ordering::Release);
        Self {
            interval,
            last_activity_ns,
            awaiting_pong: false,
            pong_sleep: Box::pin(tokio::time::sleep_until(far_future())),
            counters,
        }
    }

    pub fn record_activity(&self) {
        self.last_activity_ns.store(util::wall_ns(), Ordering::Release);
    }

    pub fn activity_elapsed(&self) -> Duration {
        let now = util::wall_ns();
        let last = self.last_activity_ns.load(Ordering::Acquire);
        Duration::from_millis(now.saturating_sub(last) / 1_000_000)
    }

    pub fn awaiting_pong(&self) -> bool {
        self.awaiting_pong
    }

    /// Start the pong timeout timer.
    pub fn start_pong_timer(&mut self, timeout: Duration) {
        self.awaiting_pong = true;
        self.pong_sleep.as_mut().reset(tokio::time::Instant::now() + timeout);
    }

    /// Cancel the pong timeout — called by the control lane after
    /// processing a pong timeout (whether suppressed or counted).
    /// Without this, awaiting_pong stays true and pong_sleep stays
    /// armed — the select! arm fires again immediately.
    pub fn cancel_pong_timer(&mut self) {
        self.awaiting_pong = false;
        self.pong_sleep.as_mut().reset(far_future());
    }

    /// Access counters for the control lane's timeout handler.
    pub fn counters(&self) -> &Arc<BulkCounters> {
        &self.counters
    }

    /// Handle a PONG frame inline from the Control lane's PONG interceptor.
    /// Takes explicit ControlState fields.
    pub fn handle_pong_inline(
        &mut self,
        state: &mut ControlState,
        payload: &[u8],
    ) {
        let was_degraded = state.heartbeat_miss_count > 0;

        let outcome = match verify_and_apply(
            &mut state.last_ping_nonce,
            &mut state.previous_ping_nonce,
            &mut state.heartbeat_miss_count,
            &mut state.remote_last_seen_our_seq,
            &mut state.last_pong_received,
            payload,
        ) {
            Ok(o) => o,
            Err(e) => {
                tracing::warn!(error = ?e, "PONG decode failed");
                return;
            }
        };

        match outcome {
            PongOutcome::Accepted { ping_sent_epoch_ns } => {
                self.counters.heartbeat_pongs_accepted.fetch_add(1, Ordering::Relaxed);
                self.awaiting_pong = false;
                self.pong_sleep.as_mut().reset(far_future());

                let now_ns = util::wall_ns();
                let max_rtt_ns = self.interval.as_nanos() as u64 * 2;
                if ping_sent_epoch_ns <= now_ns && (now_ns - ping_sent_epoch_ns) <= max_rtt_ns {
                    let rtt_us = (now_ns - ping_sent_epoch_ns) / 1_000;
                    tracing::debug!(rtt_us, "PONG accepted — RTT measured");
                }
            }
            PongOutcome::AcceptedStale => {
                self.counters.heartbeat_stale_pongs.fetch_add(1, Ordering::Relaxed);
                self.awaiting_pong = false;
                self.pong_sleep.as_mut().reset(far_future());
            }
            PongOutcome::Discarded => {
                self.counters.heartbeat_unsolicited_pongs.fetch_add(1, Ordering::Relaxed);
            }
        }

        if was_degraded && state.heartbeat_miss_count == 0 {
            tracing::info!("heartbeat: recovered from degraded state");
        }
    }

}

fn far_future() -> tokio::time::Instant {
    tokio::time::Instant::now() + Duration::from_secs(86400 * 365 * 30)
}
