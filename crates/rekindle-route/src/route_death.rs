//! One identifier-free record per own-route death (r7 design §4.6, "one live
//! measurement"): does a route's death start on the leg next to us?
//!
//! veilid-core 0.5.8 (the pinned fork, ADR 0017) gives no per-leg signal
//! through its API: `RouteChange` carries only route ids
//! (`veilid_api/types/veilid_state.rs:272-280`); the public API has no route
//! test or route stats call (`veilid_api/api.rs:763-920` allocates, imports
//! and releases only); and its own loopback tests are end to end, so even
//! internally a death blames every hop of the route at once
//! (`route_spec_store/route_stats.rs:377-385`, `:716-735`). Its one
//! first-hop signal, `failed_to_send` ("names us and the first hop",
//! `route_stats.rs:10-11`), reaches only its own debug log, beside the
//! route's key (`route_stats.rs:728`). So this records what can be known
//! here: the class, the hop count, the route's age, how it died, its
//! self-test history and whether our node was ready at the time.
//!
//! Threat model L2: no route id, node id or key is ever a field; counts and
//! durations only. It is a local log line, not telemetry.

use std::time::Duration;

use rekindle_records::port::RouteClass;
use tokio::time::Instant;

/// How an own route died.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeathCause {
    /// Veilid listed it in `RouteChange.dead_routes`.
    VeilidReported,
    /// Our own self-test declared it dead (plan RH1r-c O3) first.
    SelfTest,
}

impl DeathCause {
    fn as_str(self) -> &'static str {
        match self {
            Self::VeilidReported => "veilid",
            Self::SelfTest => "self_test",
        }
    }
}

/// The self-test history of the live route of one class.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TestTally {
    /// Consecutive failed self-tests.
    pub failures: u32,
    /// Self-tests passed over the route's life.
    pub passes: u32,
    /// When it last passed one.
    pub last_ok: Option<Instant>,
}

impl TestTally {
    /// A self-test of the route came back `ok` at `now`.
    pub fn record(&mut self, now: Instant, ok: bool) {
        if ok {
            self.failures = 0;
            self.passes = self.passes.saturating_add(1);
            self.last_ok = Some(now);
        } else {
            self.failures = self.failures.saturating_add(1);
        }
    }
}

/// One own-route death, every field a count, a duration or a class.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteDeath {
    pub class: RouteClass,
    pub cause: DeathCause,
    /// The route's hop count, when the network can say.
    pub hops: Option<usize>,
    /// How long the route lived.
    pub age: Duration,
    /// Consecutive failed self-tests at death.
    pub failures: u32,
    /// Self-tests it passed in its life.
    pub passes: u32,
    /// Since it last passed a self-test; `None` when it never did.
    pub since_ok: Option<Duration>,
    /// Whether our node was ready (PublicInternet published) at death.
    pub node_ready: bool,
}

impl RouteDeath {
    /// The death of a route that landed at `born`, at `now`.
    #[must_use]
    pub fn new(
        class: RouteClass,
        cause: DeathCause,
        hops: Option<usize>,
        born: Instant,
        tally: TestTally,
        node_ready: bool,
        now: Instant,
    ) -> Self {
        Self {
            class,
            cause,
            hops,
            age: now.saturating_duration_since(born),
            failures: tally.failures,
            passes: tally.passes,
            since_ok: tally.last_ok.map(|at| now.saturating_duration_since(at)),
            node_ready,
        }
    }

    /// The fields as `name=value` pairs, in log order.
    #[must_use]
    pub fn fields(&self) -> String {
        format!(
            "class={:?} cause={} hops={} age_s={} failures={} passes={} since_ok_s={} node_ready={}",
            self.class,
            self.cause.as_str(),
            self.hops.map_or_else(|| "unknown".to_owned(), |h| h.to_string()),
            self.age.as_secs(),
            self.failures,
            self.passes,
            self.since_ok
                .map_or_else(|| "never".to_owned(), |d| d.as_secs().to_string()),
            self.node_ready,
        )
    }

    /// The one `info` line per own-route death.
    pub fn log(&self) {
        tracing::info!(death = %self.fields(), "own route died");
    }
}

#[cfg(test)]
#[path = "route_death_tests.rs"]
pub(crate) mod tests;
