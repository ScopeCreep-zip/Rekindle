//! The friend maintenance task (plan E2.1), one per unlock on both hosts:
//! every 30 s evicting the peer routes past their TTL (the route cache's half of the old route watchdog); every
//! sixth tick (~3 min) the expiry of stale requests and invites. The ticks
//! are tokio time (paused under netsim): C9.2's `Clock` reads the time, it
//! does not wait. Sends are the outbox's (plan E2.5), and a removed friend
//! is forgotten on its `Unfriended` row's settle event, not here.
//!
//! Friend-presence staleness (plan E2.5, D23) is a local TTL timer of its
//! own: every [`STALE_TICK`] it marks offline the friends whose last
//! heartbeat is past the threshold, reading nothing from the DHT.
//!
//! The prekey maintenance (plan E2.4, B1) is a task of its own. It starts
//! only when the own-records stage says our prekey record is held,
//! reconciled and filled (a oneshot sent after `fill_record`; audit D-7:
//! it used to poll the lease, which is set before the fill, and tick at
//! once, so it could mint a second set beside the stage's), and its first
//! pass is one [`PREKEY_TICK`] later; then every tick it plans, mints and
//! publishes what our record needs.

use std::sync::Arc;
use std::time::Duration;

use rekindle_transport::operations::friend;
use tokio_util::sync::CancellationToken;

use super::HostFriends;
use crate::daemon::dispatch::DaemonContext;

/// The tick.
const TICK: Duration = Duration::from_secs(30);
/// Expiry runs every this many ticks.
const EXPIRY_EVERY: u64 = 6;
/// The staleness timer's tick.
const STALE_TICK: Duration = Duration::from_secs(30);
/// The prekey maintenance's tick: its rules run in days (B1), and a
/// consumed slot is refilled at once by the accept that used it (B8).
const PREKEY_TICK: Duration = Duration::from_secs(60 * 60);

/// Start the task in `scope` (the unlock's).
///
/// # Errors
/// The scope is closed (the unlock is ending).
pub(crate) fn start(
    ctx: &DaemonContext,
    scope: &Arc<rekindle_lifecycle::SessionScope>,
) -> Result<(), rekindle_lifecycle::ScopeClosed> {
    let host = HostFriends::of(ctx);
    let stale = host.clone();
    scope.spawn_with_token("friend maintenance", move |stop| run(host, stop))?;
    scope.spawn_with_token("friend presence staleness", move |stop| {
        run_staleness(stale, stop)
    })
}

/// Every [`STALE_TICK`], mark the friends with a stale heartbeat offline.
async fn run_staleness(host: HostFriends, stop: CancellationToken) {
    let mut ticks = tokio::time::interval(STALE_TICK);
    ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            () = stop.cancelled() => return,
            _ = ticks.tick() => {}
        }
        rekindle_presence::check_stale_friend_presences(&host);
    }
}

/// Start the prekey maintenance in `scope` (the unlock's); it waits for
/// `ready` (the own-records stage's signal that our record is filled).
///
/// # Errors
/// The scope is closed (the unlock is ending).
pub(crate) fn start_prekeys(
    ctx: &DaemonContext,
    scope: &Arc<rekindle_lifecycle::SessionScope>,
    ready: tokio::sync::oneshot::Receiver<()>,
) -> Result<(), rekindle_lifecycle::ScopeClosed> {
    let host = HostFriends::of(ctx);
    scope.spawn_with_token("prekey maintenance", move |stop| {
        run_prekeys(ready, stop, move || {
            let host = host.clone();
            async move {
                if let Err(e) = rekindle_transport::operations::prekeys::maintain(&host).await {
                    tracing::warn!(error = %e, "prekey maintenance failed");
                }
            }
        })
    })
}

/// Wait for `ready`, then run `pass` every [`PREKEY_TICK`], the first one
/// tick after `ready`. A `ready` dropped unsent (the stage did not hold
/// our record) ends the task: there is nothing to maintain this unlock.
async fn run_prekeys<P, F>(
    ready: tokio::sync::oneshot::Receiver<()>,
    stop: CancellationToken,
    mut pass: P,
) where
    P: FnMut() -> F,
    F: std::future::Future<Output = ()>,
{
    tokio::select! {
        () = stop.cancelled() => return,
        held = ready => if held.is_err() {
            tracing::debug!("prekey maintenance: no prekey record held this unlock");
            return;
        },
    }
    let mut ticks =
        tokio::time::interval_at(tokio::time::Instant::now() + PREKEY_TICK, PREKEY_TICK);
    ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            () = stop.cancelled() => return,
            _ = ticks.tick() => {}
        }
        pass().await;
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
            // Reset requests either way live as long as the hold they wait on.
            let ttl = rekindle_idempotency::intent_gate::BACKEND_HOLD_TTL_MS;
            if let Err(e) = friend::expire_resets(&host, ttl).await {
                tracing::warn!(error = %e, "expiring stale session resets failed");
            }
            match friend::expire_stale_invites(&host).await {
                Ok(0) => {}
                Ok(expired) => tracing::info!(expired, "expired stale friend invites"),
                Err(e) => tracing::warn!(error = %e, "expiring stale invites failed"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    fn counting() -> (Arc<AtomicUsize>, impl FnMut() -> std::future::Ready<()>) {
        let passes = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&passes);
        (passes, move || {
            counter.fetch_add(1, Ordering::SeqCst);
            std::future::ready(())
        })
    }

    async fn settle(by: Duration) {
        tokio::time::advance(by).await;
        for _ in 0..8 {
            tokio::task::yield_now().await;
        }
    }

    /// Audit D-7: no pass runs before the own-records stage has filled our
    /// record, however long it takes, and the first one is a whole tick
    /// after it says so.
    #[tokio::test(start_paused = true)]
    async fn the_maintenance_cannot_mint_before_the_stage_finishes() {
        let (passes, pass) = counting();
        let (ready, wait) = tokio::sync::oneshot::channel();
        let stop = CancellationToken::new();
        let task = tokio::spawn(run_prekeys(wait, stop.clone(), pass));
        settle(PREKEY_TICK * 5).await;
        assert_eq!(passes.load(Ordering::SeqCst), 0, "ran before the stage");
        ready.send(()).unwrap();
        settle(Duration::ZERO).await;
        settle(PREKEY_TICK.checked_sub(Duration::from_secs(1)).unwrap()).await;
        assert_eq!(passes.load(Ordering::SeqCst), 0, "ran at once");
        settle(Duration::from_secs(1)).await;
        assert_eq!(passes.load(Ordering::SeqCst), 1);
        settle(PREKEY_TICK).await;
        assert_eq!(passes.load(Ordering::SeqCst), 2);
        stop.cancel();
        task.await.unwrap();
    }

    /// A stage that did not hold our record drops the signal: the task ends
    /// without a pass.
    #[tokio::test(start_paused = true)]
    async fn no_record_held_means_no_maintenance() {
        let (passes, pass) = counting();
        let (ready, wait) = tokio::sync::oneshot::channel::<()>();
        drop(ready);
        run_prekeys(wait, CancellationToken::new(), pass).await;
        assert_eq!(passes.load(Ordering::SeqCst), 0);
    }
}
