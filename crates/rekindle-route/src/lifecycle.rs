//! Event-driven route lifecycle policy.
//!
//! Routes are NOT rotated on a timer: veilid-core manages route health
//! itself (its private_route_management task tests routes and reports
//! death via `RouteChange`), and a fixed rotation interval is also a
//! deterministic-timing fingerprint. A route lives until Veilid names
//! it dead or attachment is lost; death triggers an immediate heal
//! (forget → allocate → republish). These constants and the `HealGate`
//! define that policy identically for every process (Tauri host,
//! daemon/CLI transport) — backend-owns-policy.

use std::time::{Duration, Instant};

/// Cadence of the attached-but-routeless watchdog — the only timer in
/// the route lifecycle. It backstops missed heals (allocation failures,
/// startup races, reattach), never rotates a live route.
pub const ROUTE_WATCHDOG_INTERVAL: Duration = Duration::from_secs(30);

/// Minimum spacing between dead-route heal attempts. A flapping
/// network can emit `RouteChange` bursts; within the cooldown the heal
/// is skipped (state is already forgotten) and the watchdog covers it.
pub const HEAL_COOLDOWN: Duration = Duration::from_secs(10);

/// Backstop TTL for cached PEER route blobs. Peers no longer rotate on
/// a timer; their blobs stay valid until dead-remote-route events or
/// send failures invalidate them. This guards against silently-dead
/// entries only.
pub const PEER_ROUTE_CACHE_MAX_AGE: Duration = Duration::from_secs(900);

/// Flap debounce for dead-route heals.
#[derive(Debug, Clone)]
pub struct HealGate {
    last_attempt: Option<Instant>,
    cooldown: Duration,
}

impl HealGate {
    pub fn new(cooldown: Duration) -> Self {
        Self {
            last_attempt: None,
            cooldown,
        }
    }

    /// Record and admit a heal attempt: `true` when no attempt ran
    /// within the cooldown (the attempt is recorded), `false` when one
    /// did (caller skips; the watchdog backstops).
    pub fn try_begin(&mut self, now: Instant) -> bool {
        if let Some(last) = self.last_attempt {
            if now.saturating_duration_since(last) < self.cooldown {
                return false;
            }
        }
        self.last_attempt = Some(now);
        true
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::HealGate;

    #[test]
    fn heal_gate_blocks_within_cooldown() {
        let start = Instant::now();
        let mut gate = HealGate::new(Duration::from_secs(10));
        assert!(gate.try_begin(start), "first attempt always admitted");
        assert!(
            !gate.try_begin(start + Duration::from_secs(9)),
            "within cooldown must be blocked"
        );
    }

    #[test]
    fn heal_gate_allows_after_cooldown() {
        let start = Instant::now();
        let mut gate = HealGate::new(Duration::from_secs(10));
        assert!(gate.try_begin(start));
        assert!(gate.try_begin(start + Duration::from_secs(10)));
        // The admitted attempt re-arms the cooldown.
        assert!(!gate.try_begin(start + Duration::from_secs(19)));
    }
}
