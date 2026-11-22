//! The bounds the L-FR2 scenarios hold, derived from the constants that
//! govern them (lead's decision after the trace of `a7ac7764a`): never
//! literals chosen before a trace.

use std::time::Duration;

use rekindle_netsim::latency;
use rekindle_route::contexts::RouteContextSpec;
use rekindle_route::liveness::{PUBLISH_DELAY, SILENCE_TRIGGER};

/// The owner's evidence loop wakes once a second (`rekindle-transport`
/// `liveness/owner.rs` `EVIDENCE_TICK`): a due rotation starts up to this late.
const EVIDENCE_TICK: Duration = Duration::from_secs(1);

/// One app call's deadline over our safety route (`call_direct` sends
/// over `rc_safe`; Veilid 0.5.8 `routed_timeout`, [`rekindle_netsim::call_timeout`]).
pub fn call_timeout() -> Duration {
    rekindle_netsim::call_timeout(&RouteContextSpec::rc_safe().safety_profile())
}

/// The latest a route that silently stopped delivering (its online
/// detector deferring Veilid's keepalive, so Veilid reports nothing) is
/// rotated on evidence by its owner (plan RV1), from the moment it died:
/// its owner awaits a peer's answer, and nothing arrived over the route
/// for [`SILENCE_TRIGGER`] (120 s), then one evidence tick.
pub fn worst_detection() -> Duration {
    SILENCE_TRIGGER + EVIDENCE_TICK
}

/// From the rotation to the replacement on the network: the replacement's
/// test at allocation (one routed call at most, Veilid 0.5.8
/// `veilid_api/api.rs:831`), the republish's debounce [`PUBLISH_DELAY`]
/// (the pacing gap has passed since the unlock's publish), and one DHT set
/// at its slowest ([`latency::SET`]).
pub fn republish() -> Duration {
    call_timeout() + PUBLISH_DELAY + latency::SET.max
}

/// From the replacement on the network to the accept applied: the
/// accepter's watch on the requester's profile hears it
/// ([`latency::WATCH`] at its slowest), brings the waiting row forward
/// (fix (b)) and the row's one call lands within [`call_timeout`].
pub fn after_republish() -> Duration {
    latency::WATCH.max + call_timeout()
}

/// The margin over the sum: the loop tick of the row's forward move and of
/// the scenario's poll.
pub const MARGIN: Duration = Duration::from_secs(2);
