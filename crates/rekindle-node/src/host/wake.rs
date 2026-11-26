//! The Host's wake trigger (plan VU decision 8, synthesis #7). Veilid 0.5.8
//! dropped its tick-lag and leaving-Degraded inspections: a watched record
//! that changed while the machine slept surfaces only through the fallback
//! inspection's silence timer, up to 300 s later (`max_watch_inspect_interval_ms`,
//! `veilid_config.rs`), and a sleep with no Offline transition fires no
//! readiness edge. So the Host notices the sleep itself: a tick every
//! [`WAKE_TICK`] on tokio's monotonic clock, which does not advance across a
//! macOS sleep, compared against the wall clock (`Clock` port); a gap of
//! [`WAKE_GAP`] or more means the machine slept, and the Host runs the
//! friends' heal and the communities' full sweep. One mechanism on desktop,
//! daemon and mobile, no per-OS power API; the precedent is Tailscale
//! netmon's wall-time-jump wake check.

use std::sync::Arc;
use std::time::Duration;

use rekindle_utils::Clock;

use crate::daemon::dispatch::DaemonContext;

/// The detector's tick, on the monotonic clock.
pub(crate) const WAKE_TICK: Duration = Duration::from_secs(5);

/// A wall-clock gap between two ticks this long or longer is a sleep.
pub(crate) const WAKE_GAP: Duration = Duration::from_secs(30);

/// Watches one clock's ticks for a sleep.
pub(crate) struct WakeDetector {
    clock: Arc<dyn Clock>,
    last_ms: u64,
}

impl WakeDetector {
    pub(crate) fn new(clock: Arc<dyn Clock>) -> Self {
        let last_ms = clock.now_ms();
        Self { clock, last_ms }
    }

    /// A tick: whether the wall clock moved [`WAKE_GAP`] or more since the
    /// last one. A clock set back is no sleep.
    pub(crate) fn tick(&mut self) -> bool {
        let now = self.clock.now_ms();
        let gap = now.saturating_sub(self.last_ms);
        self.last_ms = now;
        u128::from(gap) >= WAKE_GAP.as_millis()
    }
}

/// Run the detector in the unlock's `scope` (a lock ends it): each wake
/// heals the friends (watches, profiles, chains) and re-runs the
/// communities' network stage.
pub(crate) fn start(ctx: &DaemonContext, scope: &Arc<rekindle_lifecycle::SessionScope>) {
    let friends = super::adapters::friends::HostFriends::of(ctx);
    let governance = crate::daemon::dispatch::adapter(ctx);
    let mut detector = WakeDetector::new(Arc::clone(&ctx.clock));
    scope.spawn_with_token_or_drop("wake trigger", move |stop| async move {
        let mut ticks = tokio::time::interval(WAKE_TICK);
        ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        ticks.tick().await;
        while stop.run_until_cancelled(ticks.tick()).await.is_some() {
            if !detector.tick() {
                continue;
            }
            tracing::info!("woke from a sleep: healing friends and communities");
            if stop
                .run_until_cancelled(async {
                    friends.heal().await;
                    governance.heal_communities().await;
                })
                .await
                .is_none()
            {
                return;
            }
            // The heal's own time is no sleep.
            detector.tick();
        }
    });
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    struct Manual(AtomicU64);

    impl Clock for Manual {
        fn now_ms(&self) -> u64 {
            self.0.load(Ordering::SeqCst)
        }

        fn monotonic(&self) -> std::time::Instant {
            std::time::Instant::now()
        }
    }

    #[test]
    fn a_wall_clock_jump_between_ticks_is_a_sleep() {
        let clock = Arc::new(Manual(AtomicU64::new(1_000_000)));
        let mut detector = WakeDetector::new(Arc::clone(&clock) as Arc<dyn Clock>);
        clock.0.fetch_add(5_000, Ordering::SeqCst);
        assert!(!detector.tick(), "an on-time tick");
        clock.0.fetch_add(29_999, Ordering::SeqCst);
        assert!(!detector.tick(), "under the gap");
        clock.0.fetch_add(30_000, Ordering::SeqCst);
        assert!(detector.tick(), "a sleep");
        clock.0.fetch_sub(60_000, Ordering::SeqCst);
        assert!(!detector.tick(), "a clock set back is no sleep");
    }
}
