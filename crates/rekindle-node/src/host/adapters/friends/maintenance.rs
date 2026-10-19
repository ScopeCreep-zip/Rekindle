//! The friend maintenance task (plan E2.1), one per unlock on both hosts:
//! every 30 s the pending-message retry, then forgetting the removed
//! friends nothing is queued to any more; every sixth tick (~3 min) the
//! expiry of stale requests and invites.

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
