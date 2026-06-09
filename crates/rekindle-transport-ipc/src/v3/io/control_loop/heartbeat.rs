//! Heartbeat state machine — owns timer and miss-count lifecycle.
//!
//! Two-generation nonce model: SessionContext is the SOLE AUTHORITY
//! for nonce state (last_ping_nonce, previous_ping_nonce). HeartbeatState
//! owns ONLY timer state (awaiting_pong, pong_sleep) and miss tracking.
//! No nonce fields in HeartbeatState — eliminates sync hazard entirely.
//!
//! A PONG is valid if it matches either the current or previous nonce
//! generation in SessionContext. Stale PONGs (previous generation) reset
//! the miss count — the peer IS alive, the response was delayed.
//! Unsolicited PONGs (matching neither) are logged and discarded.
//!
//! Pong timeout suppression: if the connection has received ANY frame
//! (bulk data, control, audit) within the pong timeout window, the peer
//! is demonstrably alive. A pong timeout in this case means the peer's
//! control loop was too busy to process our PING — not that the peer is
//! dead. The timeout increments the miss counter but activity resets it.
//!
//! Only miss_limit CONSECUTIVE timeouts with NO activity between them
//! are connection-fatal.

use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use tokio::time::Sleep;

use crate::v3::bulk::counters::BulkCounters;
use crate::v3::codec::channel::ping as ping_codec;
use crate::v3::context::{OutboundFrame, SessionContext};
use crate::v3::handlers::channel::pong::{verify_and_apply, PongOutcome};
use crate::v3::io::control_loop::drain::DrainContext;
use crate::v3::io::read_task::SessionOutcome;
use crate::v3::wire::frame_kind::ChannelKind;

use super::audit_reorder::AuditReorderBuffer;
use super::drain;
use super::timers;
use super::util;

/// All heartbeat-related state for one connection.
/// Nonce state lives exclusively in SessionContext — not duplicated here.
///
/// `last_activity_ns` is an `Arc<AtomicU64>` shared with the read task.
/// The read task writes to it on every frame received from the socket —
/// even when the control loop is blocked in drain_outbound AEAD. This
/// ensures the heartbeat sees peer activity regardless of control loop
/// load. The control loop also writes to it on RecvFrame/BulkDecrypted.
pub(super) struct HeartbeatState {
    interval: Duration,
    pong_timeout: Duration,
    miss_limit: u32,
    /// Shared with read task. Epoch nanos of last frame received.
    /// Updated by both read task (socket frames) and control loop
    /// (processed frames). Read by heartbeat tick/pong_timeout.
    last_activity_ns: Arc<std::sync::atomic::AtomicU64>,
    awaiting_pong: bool,
    /// Pinned Sleep future — reset in-place when PING is sent.
    /// When not awaiting, set to far_future (never fires).
    pub pong_sleep: Pin<Box<Sleep>>,
    /// Observability counters — shared across all connections.
    counters: Arc<BulkCounters>,
}

impl HeartbeatState {
    pub fn new(
        interval: Duration,
        pong_timeout: Duration,
        miss_limit: u32,
        counters: Arc<BulkCounters>,
        last_activity_ns: Arc<std::sync::atomic::AtomicU64>,
    ) -> Self {
        last_activity_ns.store(util::wall_ns(), std::sync::atomic::Ordering::Release);
        Self {
            interval,
            pong_timeout,
            miss_limit,
            last_activity_ns,
            awaiting_pong: false,
            pong_sleep: Box::pin(tokio::time::sleep_until(far_future())),
            counters,
        }
    }

    /// Record that application-level activity occurred (frame received,
    /// bulk chunk processed, any evidence the peer is alive).
    /// Also callable from the read task via the shared atomic.
    pub fn record_activity(&self) {
        self.last_activity_ns.store(util::wall_ns(), std::sync::atomic::Ordering::Release);
    }

    /// Elapsed time since last activity, in milliseconds.
    fn activity_elapsed_ms(&self) -> u64 {
        let now = util::wall_ns();
        let last = self.last_activity_ns.load(std::sync::atomic::Ordering::Acquire);
        now.saturating_sub(last) / 1_000_000
    }

    /// Elapsed time since last activity, as Duration.
    fn activity_elapsed(&self) -> Duration {
        Duration::from_millis(self.activity_elapsed_ms())
    }

    /// Handle a PONG frame. Called by the control loop's PONG interceptor
    /// with access to both &mut SessionContext and &mut self.
    ///
    /// Delegates nonce verification and SessionContext mutation to
    /// verify_and_apply — single source of truth. This method
    /// additionally cancels HeartbeatState timers and updates counters.
    pub fn handle_pong(
        &mut self,
        ctx: &mut SessionContext,
        payload: &[u8],
    ) {
        let outcome = match verify_and_apply(ctx, payload) {
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

                // RTT computation from the echoed PING timestamp.
                // Per SCTP §8.3: validate sent_at + hbinterval + rto.
                // Reject PONGs with impossible timestamps (future or
                // older than 2× heartbeat interval).
                let now_ns = util::wall_ns();
                let max_rtt_ns = self.interval.as_nanos() as u64 * 2;
                if ping_sent_epoch_ns <= now_ns && (now_ns - ping_sent_epoch_ns) <= max_rtt_ns {
                    let rtt_us = (now_ns - ping_sent_epoch_ns) / 1_000;
                    tracing::debug!(rtt_us, "PONG accepted — RTT measured");
                } else {
                    tracing::warn!(
                        ping_sent_epoch_ns, now_ns, max_rtt_ns,
                        "PONG accepted — echoed timestamp outside valid window, RTT discarded"
                    );
                }
            }
            PongOutcome::AcceptedStale => {
                tracing::info!("PONG accepted — stale (previous generation), peer alive but slow");
                self.counters.heartbeat_stale_pongs.fetch_add(1, Ordering::Relaxed);
                // Peer IS alive — it responded to the previous PING.
                // Reset awaiting_pong and timer. Miss count already reset
                // by verify_and_apply. last_ping_nonce stays valid — the
                // current generation nonce is still outstanding.
                // Do NOT call record_pong_received — RTT would be invalid
                // for a stale PONG (inflated by one heartbeat interval).
                self.awaiting_pong = false;
                self.pong_sleep.as_mut().reset(far_future());
            }
            PongOutcome::Discarded => {
                self.counters.heartbeat_unsolicited_pongs.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    /// Whether we are currently awaiting a PONG response.
    pub fn awaiting_pong(&self) -> bool {
        self.awaiting_pong
    }

    /// Handle pong timeout firing. Returns Some(SessionOutcome) if fatal.
    ///
    /// Activity-aware: if ANY frame was received within the pong timeout
    /// window, the peer is demonstrably alive (it sent us data). The
    /// timeout means the peer's control loop was too busy processing
    /// bulk data to respond to our PING — not that the peer is dead.
    /// In this case, reset the miss count instead of incrementing it.
    pub fn handle_pong_timeout(&mut self, ctx: &mut SessionContext) -> Option<SessionOutcome> {
        self.counters.heartbeat_pong_timeouts.fetch_add(1, Ordering::Relaxed);

        // Activity-based suppression: if we received ANY frame from the
        // peer within the pong timeout window, the peer is alive. The
        // PONG was lost because the peer's control loop was processing
        // bulk data. Do NOT count this as a miss.
        if self.activity_elapsed() < self.pong_timeout {
            tracing::debug!(
                last_activity_ms = self.activity_elapsed().as_millis(),
                pong_timeout_ms = self.pong_timeout.as_millis(),
                "pong timeout suppressed — peer active within timeout window"
            );
            ctx.reset_heartbeat_miss();
            // Rotate nonces so the in-flight PONG can still arrive as stale
            ctx.rotate_ping_nonce();
            self.awaiting_pong = false;
            self.pong_sleep.as_mut().reset(far_future());
            return None;
        }

        tracing::debug!(
            miss_count = ctx.heartbeat_miss_count(),
            miss_limit = self.miss_limit,
            last_activity_ms = self.activity_elapsed().as_millis(),
            "pong timeout — no response and no activity within deadline"
        );

        ctx.increment_heartbeat_miss();

        // Rotate nonce generations in SessionContext.
        // The in-flight PONG gets one more cycle to arrive as "stale but valid".
        ctx.rotate_ping_nonce();
        self.awaiting_pong = false;
        self.pong_sleep.as_mut().reset(far_future());

        if ctx.heartbeat_miss_count() >= self.miss_limit {
            tracing::error!(
                miss_count = ctx.heartbeat_miss_count(),
                miss_limit = self.miss_limit,
                "heartbeat dead — miss limit reached with no activity"
            );
            Some(SessionOutcome::HeartbeatTimeout {
                last_ping_at: util::wall_ns(),
            })
        } else {
            None
        }
    }

    /// Handle heartbeat tick. Sends PING if idle, sweeps expired requests,
    /// checks deadlines. Returns Some(SessionOutcome) on fatal condition.
    pub async fn tick(
        &mut self,
        ctx: &mut SessionContext,
        drain_ctx: &DrainContext<'_>,
        outbound_reorder: &mut AuditReorderBuffer,
    ) -> Option<SessionOutcome> {
        if self.activity_elapsed() >= self.interval && !self.awaiting_pong {
            let nonce = util::rand_nonce();
            let ping = ping_codec::PingPayload {
                ping_nonce: nonce,
                sender_epoch_ns: util::wall_ns(),
                last_seen_remote_seq: ctx.recv_last_seq(),
            };
            // SessionContext is the sole authority for nonce state
            ctx.set_last_ping_nonce(nonce);
            ctx.push_outbound(OutboundFrame::Channel {
                kind: ChannelKind::Ping,
                payload: ping_codec::encode(&ping),
            });
            self.awaiting_pong = true;
            self.pong_sleep.as_mut().reset(tokio::time::Instant::now() + self.pong_timeout);
            if let Some(outcome) = drain::drain_outbound(ctx, drain_ctx, outbound_reorder).await {
                return Some(outcome);
            }
        }
        sweep_expired_requests(ctx);
        timers::check_deadlines(ctx)
    }
}

fn far_future() -> tokio::time::Instant {
    // tokio::time::Instant::far_future() is pub(crate) — not callable.
    // 30 years matches tokio's internal definition. 1 day was too short
    // for connections that may run for weeks.
    tokio::time::Instant::now() + Duration::from_secs(86400 * 365 * 30)
}

fn sweep_expired_requests(ctx: &mut SessionContext) {
    let now = Instant::now();
    let expired: Vec<uuid::Uuid> = ctx.pending_requests_iter()
        .filter(|(_, req)| req.is_expired(now))
        .map(|(id, _)| *id)
        .collect();
    for id in expired {
        ctx.resolve_pending_request(&id);
    }
}
