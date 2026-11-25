//! A route class's outage (plan RV1g, F9 A1/A3/A4): when the class went
//! down, how its replacements fail, and what the shells and peers are told.
//!
//! veilid-core 0.5.8 (the pinned fork, ADR 0017): a new route is tested
//! before it is handed out, and one that fails its test is `TryAgain`
//! ("allocated route failed to test", `veilid_api/api.rs:829-836`). That is
//! retried for ever (never a stop); after [`SLOW_AFTER_FAILED_TESTS`] in a
//! row the retry steps to [`SLOW_BACKOFF`]. A failed test spends no budget
//! and records no death (only a live route's loss does), so it never feeds
//! the death-rate warning.
//!
//! The shells are told once on the down transition and once on the way up
//! ([`OwnRouteDownSet`], followed by the Host), never per attempt. Peers
//! are told only after a death Veilid reported (`RouteChange.dead_routes`),
//! never after our own rotation, and only once the General class has been
//! down past [`WITHDRAW_GRACE`]: the route is then withdrawn (an empty blob
//! under the next seq), so readers stop importing a dead blob.

use std::time::Duration;

use parking_lot::Mutex;
use rekindle_records::port::{RouteClass, WallMs};
use rekindle_types::subscription_events::{OwnRouteClass, OwnRouteDownSet, OwnRouteOutage};
use tokio::sync::watch;
use tokio::time::Instant;

use crate::per_class::PerClass;

/// How often "own route still down" repeats after its 10th attempt.
pub const STILL_DOWN_EVERY: Duration = Duration::from_secs(60);
/// A class down this long is warned about, once.
pub const DOWN_WARN_AFTER: Duration = Duration::from_secs(60);
/// How long the General class stays down after a Veilid-reported death
/// before its published route is withdrawn.
pub const WITHDRAW_GRACE: Duration = Duration::from_secs(30);
/// Consecutive failed tests after which the retry steps to [`SLOW_BACKOFF`].
pub const SLOW_AFTER_FAILED_TESTS: u32 = 20;
/// The retry once the tests keep failing.
pub const SLOW_BACKOFF: Duration = Duration::from_secs(60);

/// Veilid's reason for a route that failed its test (`api.rs:835`).
const FAILED_TEST: &str = "failed to test";

#[derive(Default)]
struct Track {
    /// When the class went down: the monotonic instant and the epoch ms.
    since: Option<(Instant, u64)>,
    attempts: u32,
    /// Consecutive "failed to test" answers.
    failed_tests: u32,
    shown: bool,
    warned: bool,
    noted_at: Option<Instant>,
    /// The loss was Veilid's report, not our rotation.
    veilid_dead: bool,
    withdrawn: bool,
}

/// What a failed attempt means for the allocation loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Failure {
    /// The retry steps to [`SLOW_BACKOFF`].
    pub slow: bool,
    /// Withdraw the published General route now.
    pub withdraw: bool,
    /// How long until a withdrawal falls due, if one is pending.
    pub withdraw_in: Option<Duration>,
}

fn shown(class: RouteClass) -> OwnRouteClass {
    match class {
        RouteClass::General => OwnRouteClass::General,
        RouteClass::Media => OwnRouteClass::Media,
    }
}

/// Both classes' outages and the set the Host follows.
pub(crate) struct Downs {
    wall: WallMs,
    tracks: PerClass<Mutex<Track>>,
    set: watch::Sender<OwnRouteDownSet>,
}

impl Downs {
    pub(crate) fn new(wall: WallMs) -> Self {
        Self {
            wall,
            tracks: PerClass::new(|_| Mutex::new(Track::default())),
            set: watch::Sender::new(OwnRouteDownSet::default()),
        }
    }

    fn track(&self, class: RouteClass) -> &Mutex<Track> {
        self.tracks.get(class)
    }

    pub(crate) fn subscribe(&self) -> watch::Receiver<OwnRouteDownSet> {
        self.set.subscribe()
    }

    /// The class's live route is gone; `veilid_dead` when Veilid reported
    /// it dead (only that loss may withdraw the route).
    pub(crate) fn lost(&self, class: RouteClass, veilid_dead: bool) {
        let mut track = self.track(class).lock();
        if track.since.is_none() {
            track.since = Some((Instant::now(), (self.wall)()));
        }
        track.veilid_dead = veilid_dead;
    }

    /// An allocation of `class` failed with `reason`.
    pub(crate) fn failed(&self, class: RouteClass, reason: &str) -> Failure {
        let now = Instant::now();
        let mut track = self.track(class).lock();
        let (at, since_ms) = *track.since.get_or_insert_with(|| (now, (self.wall)()));
        let down = now.saturating_duration_since(at);
        track.attempts = track.attempts.saturating_add(1);
        if reason.contains(FAILED_TEST) {
            track.failed_tests = track.failed_tests.saturating_add(1);
        } else {
            track.failed_tests = 0;
        }
        let note = matches!(track.attempts, 3 | 10)
            || (track.attempts > 10
                && track
                    .noted_at
                    .is_some_and(|t| now.saturating_duration_since(t) >= STILL_DOWN_EVERY));
        if note {
            track.noted_at = Some(now);
            tracing::info!(?class, attempts = track.attempts, failed_tests = track.failed_tests,
                down_s = down.as_secs(), %reason, "own route still down");
        }
        if !track.warned && down >= DOWN_WARN_AFTER {
            track.warned = true;
            tracing::warn!(?class, down_s = down.as_secs(), %reason, "own route down past a minute; nobody can reach this class");
        }
        if !track.shown {
            track.shown = true;
            let outage = OwnRouteOutage {
                since_ms,
                reason: reason.to_owned(),
            };
            self.set
                .send_modify(|set| *set.entry(shown(class)) = Some(outage));
        }
        let pending = class == RouteClass::General && track.veilid_dead && !track.withdrawn;
        let withdraw = pending && down >= WITHDRAW_GRACE;
        if withdraw {
            track.withdrawn = true;
        }
        Failure {
            slow: track.failed_tests >= SLOW_AFTER_FAILED_TESTS,
            withdraw,
            withdraw_in: (pending && !withdraw).then(|| WITHDRAW_GRACE.saturating_sub(down)),
        }
    }

    /// A new route of `class` landed: how long the class was down, if it was.
    pub(crate) fn up(&self, class: RouteClass) -> Option<Duration> {
        let down = self.track(class).lock().since.map(|(at, _)| at.elapsed());
        self.reset(class);
        down
    }

    /// The class ended its outage (up, or released): clear it.
    pub(crate) fn reset(&self, class: RouteClass) {
        let was_shown = std::mem::take(&mut *self.track(class).lock()).shown;
        if was_shown {
            self.set.send_modify(|set| *set.entry(shown(class)) = None);
        }
    }
}

#[cfg(test)]
#[path = "own_route_down_tests.rs"]
mod tests;
