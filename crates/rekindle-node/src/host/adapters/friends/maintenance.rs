//! The friend maintenance task (plan E2.1), one per unlock on both hosts:
//! every 30 s the pending-message retry, then forgetting the removed
//! friends nothing is queued to any more and evicting the peer routes past
//! their TTL (the route cache's half of the old route watchdog); every
//! sixth tick (~3 min) the expiry of stale requests and invites. The ticks
//! are tokio time (paused under netsim): C9.2's `Clock` reads the time, it
//! does not wait.
//!
//! The prekey maintenance (plan E2.4, B1) is a task of its own: once the
//! own-records stage holds our prekey record (and has reconciled it and
//! published its first set), every [`PREKEY_TICK`] it plans, mints and
//! publishes what our record needs.

use std::sync::Arc;
use std::time::Duration;

use rekindle_transport::operations::friend;
use tokio_util::sync::CancellationToken;

use super::{pending_retry::PendingRetry, HostFriends};
use crate::daemon::dispatch::DaemonContext;

/// The tick: `rekindle_sync`'s retry interval.
const TICK: Duration = Duration::from_millis(rekindle_sync::pending_retry::SYNC_LOOP_INTERVAL_MS);
/// Expiry runs every this many ticks.
const EXPIRY_EVERY: u64 = 6;
/// The prekey maintenance's tick: its rules run in days (B1), and a
/// consumed slot is refilled at once by the accept that used it (B8).
const PREKEY_TICK: Duration = Duration::from_secs(60 * 60);
/// How often the task looks for our prekey record before the own-records
/// stage has opened it.
const PREKEY_RECORD_WAIT: Duration = Duration::from_secs(2);

/// Start the task in `scope` (the unlock's).
///
/// # Errors
/// The scope is closed (the unlock is ending).
pub(crate) fn start(
    ctx: &DaemonContext,
    scope: &Arc<rekindle_lifecycle::SessionScope>,
) -> Result<(), rekindle_lifecycle::ScopeClosed> {
    let host = HostFriends::of(ctx);
    scope.spawn_with_token("friend maintenance", move |stop| run(host, stop))
}

/// Start the prekey maintenance in `scope` (the unlock's).
///
/// # Errors
/// The scope is closed (the unlock is ending).
pub(crate) fn start_prekeys(
    ctx: &DaemonContext,
    scope: &Arc<rekindle_lifecycle::SessionScope>,
) -> Result<(), rekindle_lifecycle::ScopeClosed> {
    let host = HostFriends::of(ctx);
    scope.spawn_with_token("prekey maintenance", move |stop| run_prekeys(host, stop))
}

async fn run_prekeys(host: HostFriends, stop: CancellationToken) {
    use rekindle_transport::operations::friend::FriendHost as _;
    use rekindle_transport::operations::prekeys;
    while host.own_prekey_lease().is_none() {
        tokio::select! {
            () = stop.cancelled() => return,
            () = tokio::time::sleep(PREKEY_RECORD_WAIT) => {}
        }
    }
    let mut ticks = tokio::time::interval(PREKEY_TICK);
    ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            () = stop.cancelled() => return,
            _ = ticks.tick() => {}
        }
        if let Err(e) = prekeys::maintain(&host).await {
            tracing::warn!(error = %e, "prekey maintenance failed");
        }
    }
}

async fn run(host: HostFriends, stop: CancellationToken) {
    let mut ticks = tokio::time::interval(TICK);
    ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut count: u64 = 0;
    loop {
        tokio::select! {
            () = stop.cancelled() => return,
            _ = ticks.tick() => {}
        }
        count += 1;
        let retry = PendingRetry {
            host: host.clone(),
            stop: stop.clone(),
        };
        rekindle_sync::process_pending_retry_queue(&retry, &stop).await;
        if stop.is_cancelled() {
            return;
        }
        if let Err(e) = friend::forget_removed(&host).await {
            tracing::warn!(error = %e, "forgetting removed friends failed");
        }
        if let Some(node) = host.ctx().transport.read().clone() {
            let evicted = node.peers().write().evict_stale_routes();
            if !evicted.is_empty() {
                tracing::debug!(count = evicted.len(), "evicted stale peer routes");
            }
        }
        if count.is_multiple_of(EXPIRY_EVERY) {
            if let Err(e) = friend::expire_stale_requests(&host).await {
                tracing::warn!(error = %e, "expiring stale friend requests failed");
            }
            match friend::expire_stale_invites(&host).await {
                Ok(0) => {}
                Ok(expired) => tracing::info!(expired, "expired stale friend invites"),
                Err(e) => tracing::warn!(error = %e, "expiring stale invites failed"),
            }
        }
    }
}
