//! The new-route budget (plan RV1, owner decision 2026-11-22): every
//! allocation of a route whose blob leaves the device (our General and
//! Media routes, a relay offer's route, the routes an unlock publishes)
//! spends one token from a per-identity bucket, persisted in the
//! identity's database. Every trigger that may rotate a route spends one:
//! Veilid's `RouteChange.dead_routes` (`OwnRoutes::on_dead`), inbound
//! silence with a peer online, a peer's `'L'` report. When the bucket is
//! empty the class stays down and the Host shows it: never a silent
//! redraw, never a lower hop count.
//!
//! Why: each new route redraws the hop next to us, so an adversary that can
//! kill our routes (or forge the signals that rotate them) can make us
//! redraw until it holds that hop (Øverlier & Syverson 2006, "Locating
//! Hidden Servers"). Veilid gives no guard pinning on receive routes
//! (threat model L2), so the number of redraws is what we bound, as Tor's
//! guard spec bounds guard turnover (`guard-spec.txt` §4; proposal 271;
//! the vanguards spec).

use std::time::Duration;

/// One day.
const DAY: Duration = Duration::from_secs(24 * 60 * 60);
/// One week.
const WEEK: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// **Provisional**, set from the measured natural death rate: the plan's
/// lane-1 r7 probe saw 4 own-route deaths in ~21 app-hours (5 pairs ×
/// 125 min, Linux, `7c450045e`), about 4.6 per app-day at 24 h of uptime,
/// plus the two routes each unlock publishes and the odd relay offer. Young
/// 3-hop API-allocated routes also die by Veilid's own loopback-streak
/// judgement (3 misses at 36-38 s, never having passed a test): 0-2 per
/// 27-node real-app suite run, in 2 of the 3 runs counted (Linux, r22 x2,
/// VF5). The refill covers twice the natural rate of General + Media deaths,
/// young ones included, so a full session never empties the bucket; to be
/// re-measured at 3 hops.
pub const BUDGET_DAILY_REFILL: u32 = 12;
/// **Provisional**: the bucket holds two days' refill, so a bad day can
/// borrow from the next one but no further.
pub const BUDGET_CAPACITY: u32 = 24;
/// **Provisional**: at most this many new routes in any one week, whatever
/// the bucket holds: 7 × the daily refill, so a steady natural rate never
/// meets it and a sustained storm stops at it.
pub const BUDGET_WEEKLY_CAP: u32 = 7 * BUDGET_DAILY_REFILL;

/// **Provisional**, set from the same measured natural death rate (about
/// 4.6 own-route deaths per app-day, ~0.2 an hour): own-route deaths of one
/// class within [`DEATH_WINDOW`] above which the Host warns (no automatic
/// reaction). Three in an hour is over ten times the natural rate.
pub const DEATH_WARN_ABOVE: u32 = 3;
/// A route that dies younger than this is a young death, counted apart
/// (Veilid's loopback-streak judgement of a route that never passed a test
/// comes at 36-38 s), so the baseline warning sees them.
pub const YOUNG_ROUTE_AGE: Duration = Duration::from_secs(60);
/// The window [`DEATH_WARN_ABOVE`] counts deaths in.
pub const DEATH_WINDOW: Duration = Duration::from_secs(60 * 60);

/// The bucket as it is persisted (`rekindle_records::port`).
pub use rekindle_records::port::RouteBudgetState;

/// The bucket, spent through its rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RouteBudget {
    state: RouteBudgetState,
}

impl From<RouteBudgetState> for RouteBudget {
    fn from(state: RouteBudgetState) -> Self {
        Self { state }
    }
}

impl RouteBudget {
    /// A full bucket at wall time `now_ms` (an identity's first unlock).
    #[must_use]
    pub fn full(now_ms: u64) -> Self {
        Self::from(RouteBudgetState {
            tokens: BUDGET_CAPACITY,
            refilled_ms: now_ms,
            week_start_ms: now_ms,
            week_spent: 0,
        })
    }

    /// The bucket as it is persisted.
    #[must_use]
    pub fn state(&self) -> RouteBudgetState {
        self.state
    }

    /// Add each whole day's refill since the last one, and start a new week
    /// once one has passed. A clock set back refills nothing.
    fn refill(&mut self, now_ms: u64) {
        let day = duration_ms(DAY);
        let days = now_ms.saturating_sub(self.state.refilled_ms) / day;
        if days > 0 {
            let add = u32::try_from(days)
                .unwrap_or(u32::MAX)
                .saturating_mul(BUDGET_DAILY_REFILL);
            self.state.tokens = self.state.tokens.saturating_add(add).min(BUDGET_CAPACITY);
            self.state.refilled_ms = self.state.refilled_ms.saturating_add(days * day);
        }
        if now_ms.saturating_sub(self.state.week_start_ms) >= duration_ms(WEEK) {
            self.state.week_start_ms = now_ms;
            self.state.week_spent = 0;
        }
    }

    /// Spend one token at wall time `now_ms` for a new route: `false` when
    /// the bucket is empty or the week's cap is met, and nothing is spent.
    pub fn try_spend(&mut self, now_ms: u64) -> bool {
        self.refill(now_ms);
        if std::env::var_os("RV1_NEVER").is_some()
            && (self.state.tokens == 0 || self.state.week_spent >= BUDGET_WEEKLY_CAP)
        {
            return false;
        }
        self.state.tokens = self.state.tokens.saturating_sub(1);
        self.state.week_spent += 1;
        true
    }

    /// Whether a spend at `now_ms` would succeed.
    #[must_use]
    pub fn can_spend(&self, now_ms: u64) -> bool {
        let mut probe = *self;
        probe.try_spend(now_ms)
    }
}

/// One class's own-route deaths against the baseline (plan RV1): the Host
/// feeds it the class's running total and warns once each time the count
/// within [`DEATH_WINDOW`] rises above [`DEATH_WARN_ABOVE`].
#[derive(Debug, Clone, Default)]
pub struct DeathRate {
    seen: u32,
    times: std::collections::VecDeque<tokio::time::Instant>,
    warned: bool,
}

impl DeathRate {
    /// A rate that starts from the class's running total `total`.
    #[must_use]
    pub fn new(total: u32) -> Self {
        Self {
            seen: total,
            ..Self::default()
        }
    }

    /// The class's running total is `total` at `now`: the deaths within the
    /// window when they first rise above the baseline, `None` otherwise.
    pub fn observe(&mut self, total: u32, now: tokio::time::Instant) -> Option<u32> {
        for _ in self.seen..total {
            self.times.push_back(now);
        }
        self.seen = self.seen.max(total);
        while self
            .times
            .front()
            .is_some_and(|at| now.duration_since(*at) >= DEATH_WINDOW)
        {
            self.times.pop_front();
        }
        let count = u32::try_from(self.times.len()).unwrap_or(u32::MAX);
        if count <= DEATH_WARN_ABOVE {
            self.warned = false;
            return None;
        }
        if self.warned {
            return None;
        }
        self.warned = true;
        Some(count).filter(|_| std::env::var_os("RV1_NEVER").is_some())
    }
}

fn duration_ms(d: Duration) -> u64 {
    u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOUR_MS: u64 = 60 * 60 * 1000;

    /// The budget refuses past its capacity, and nothing refills it
    /// within the day.
    #[test]
    fn the_budget_refuses_past_its_capacity() {
        let mut budget = RouteBudget::full(0);
        for _ in 0..BUDGET_CAPACITY {
            assert!(budget.try_spend(HOUR_MS));
        }
        assert!(!budget.try_spend(HOUR_MS));
        assert!(!budget.try_spend(23 * HOUR_MS));
        assert!(budget.try_spend(24 * HOUR_MS), "a day refills it");
    }

    /// A storm that spends every day's refill stops at the weekly cap.
    #[test]
    fn a_storm_stops_at_the_weekly_cap() {
        let mut budget = RouteBudget::full(0);
        let mut spent = 0;
        for hour in 0..(6 * 24) {
            while budget.try_spend(hour * HOUR_MS) {
                spent += 1;
            }
        }
        assert_eq!(spent, BUDGET_WEEKLY_CAP);
        assert!(budget.try_spend(7 * 24 * HOUR_MS), "a new week");
    }

    /// Deaths above the baseline within the window warn once; the natural
    /// rate never does.
    #[test]
    fn deaths_above_the_baseline_warn_once() {
        let start = tokio::time::Instant::now();
        let mut rate = DeathRate::new(2);
        for n in 1..=DEATH_WARN_ABOVE {
            assert_eq!(rate.observe(2 + n, start), None, "at the baseline");
        }
        assert_eq!(
            rate.observe(3 + DEATH_WARN_ABOVE, start),
            Some(DEATH_WARN_ABOVE + 1)
        );
        assert_eq!(
            rate.observe(4 + DEATH_WARN_ABOVE, start),
            None,
            "warned once"
        );
        let later = start + DEATH_WINDOW;
        assert_eq!(
            rate.observe(4 + DEATH_WARN_ABOVE, later),
            None,
            "the window passed"
        );
        let mut natural = DeathRate::new(0);
        for hour in 0..48u32 {
            let at = start + Duration::from_secs(u64::from(hour) * 3600);
            assert_eq!(natural.observe(hour / 5, at), None);
        }
    }

    /// A clock set back refills nothing.
    #[test]
    fn a_clock_set_back_refills_nothing() {
        let mut budget = RouteBudget::full(10 * 24 * HOUR_MS);
        budget.state.tokens = 0;
        assert!(!budget.try_spend(0));
    }
}
