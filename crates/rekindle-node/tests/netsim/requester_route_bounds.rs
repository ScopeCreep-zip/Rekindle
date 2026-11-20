//! The bounds the L-FR2 scenarios hold, derived from the constants that
//! govern them (lead's decision after the trace of `a7ac7764a`): never
//! literals chosen before a trace.

use std::time::Duration;

use rekindle_netsim::latency;
use rekindle_route::contexts::RouteContextSpec;
use rekindle_route::liveness::{
    DEAD_AFTER_FAILURES, PROBE_TIMEOUT, PUBLISH_DELAY, RETEST_JITTER_MS, RETEST_MIN,
    SELF_TEST_JITTER_MS, SELF_TEST_PERIOD,
};

/// The owner's self-test loop wakes once a second (`rekindle-transport`
/// `liveness/owner.rs` `SELF_TEST_TICK`): a due test starts up to this late.
const SELF_TEST_TICK: Duration = Duration::from_secs(1);

/// One app call's deadline over our safety route (`call_direct` sends
/// over `rc_safe`; Veilid 0.5.8 `routed_timeout`, [`rekindle_netsim::call_timeout`]).
pub fn call_timeout() -> Duration {
    rekindle_netsim::call_timeout(&RouteContextSpec::rc_safe().safety_profile())
}

/// The latest a silently dead route is declared dead by its owner's own
/// self-test, from the moment it died: the death falls right after a
/// passed test, so the next one comes a full period plus its jitter later
/// (40 s + 13.3 s); then [`DEAD_AFTER_FAILURES`] probes each wait out
/// [`PROBE_TIMEOUT`] (15 s), two retests apart at their longest (10 s +
/// 10 s), each start up to one loop tick late. 138.3 s + 3 ticks.
pub fn worst_detection() -> Duration {
    let retest = RETEST_MIN + Duration::from_millis(RETEST_JITTER_MS);
    SELF_TEST_PERIOD
        + Duration::from_millis(SELF_TEST_JITTER_MS)
        + (PROBE_TIMEOUT + SELF_TEST_TICK) * DEAD_AFTER_FAILURES
        + retest * (DEAD_AFTER_FAILURES - 1)
}

/// From the death's declaration to the replacement on the network: the
/// replacement's own self-test (at most [`PROBE_TIMEOUT`]), the
/// republish's debounce [`PUBLISH_DELAY`] (the pacing gap has passed: the
/// earliest declaration is past `PUBLISH_GAP` since the unlock's publish),
/// and one DHT set at its slowest ([`latency::SET`]).
pub fn republish() -> Duration {
    PROBE_TIMEOUT + PUBLISH_DELAY + latency::SET.max
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
