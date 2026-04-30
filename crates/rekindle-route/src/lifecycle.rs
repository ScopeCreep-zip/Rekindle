//! Route refresh cadence and dead-route recovery helpers.

use std::time::{Duration, Instant};

pub const ROUTE_REFRESH_INTERVAL: Duration = Duration::from_secs(120);

#[derive(Debug, Clone)]
pub struct RouteLifecycle {
    refresh_interval: Duration,
    last_refresh: Instant,
}

impl RouteLifecycle {
    pub fn new(now: Instant) -> Self {
        Self {
            refresh_interval: ROUTE_REFRESH_INTERVAL,
            last_refresh: now,
        }
    }

    pub fn with_interval(now: Instant, refresh_interval: Duration) -> Self {
        Self {
            refresh_interval,
            last_refresh: now,
        }
    }

    pub fn should_refresh_at(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.last_refresh) >= self.refresh_interval
    }

    pub fn mark_refreshed(&mut self, now: Instant) {
        self.last_refresh = now;
    }

    pub fn handle_dead_route(&mut self, now: Instant) {
        self.last_refresh = now.checked_sub(self.refresh_interval).unwrap_or(now);
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::RouteLifecycle;

    #[test]
    fn refresh_due_after_interval() {
        let start = Instant::now();
        let lifecycle = RouteLifecycle::with_interval(start, Duration::from_secs(120));
        assert!(!lifecycle.should_refresh_at(start + Duration::from_secs(119)));
        assert!(lifecycle.should_refresh_at(start + Duration::from_secs(120)));
    }

    #[test]
    fn dead_route_forces_immediate_refresh() {
        let start = Instant::now();
        let mut lifecycle = RouteLifecycle::with_interval(start, Duration::from_secs(120));
        lifecycle.handle_dead_route(start + Duration::from_secs(30));
        assert!(lifecycle.should_refresh_at(start + Duration::from_secs(30)));
    }
}
