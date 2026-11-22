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
    /// We rotated it on evidence that it was dead from outside (plan
    /// RV1), before Veilid reported it.
    Rotated,
}

impl DeathCause {
    fn as_str(self) -> &'static str {
        match self {
            Self::VeilidReported => "veilid",
            Self::Rotated => "rotated",
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
        node_ready: bool,
        now: Instant,
    ) -> Self {
        Self {
            class,
            cause,
            hops,
            age: now.saturating_duration_since(born),
            node_ready,
        }
    }

    /// The fields as `name=value` pairs, in log order.
    #[must_use]
    pub fn fields(&self) -> String {
        format!(
            "class={:?} cause={} hops={} age_s={} node_ready={}",
            self.class,
            self.cause.as_str(),
            self.hops
                .map_or_else(|| "unknown".to_owned(), |h| h.to_string()),
            self.age.as_secs(),
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
