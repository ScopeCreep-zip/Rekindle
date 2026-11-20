//! Live finding L-FR2, the case Linux run 8 showed: the requester's own
//! route passed its self-test every ~50 s while the accepter's
//! `FriendAccept` call timed out for 7 minutes, so the route was dead only
//! from the accepter's side. The route the accepter captured from the
//! request is stale: Ola's routes die right after her request (Veilid
//! tells her, not Tia: [`Fault::RouteDeath`]) and she publishes the
//! replacement at once, which her self-test then passes. Tia accepts over
//! the captured route; the call fails, and the accept must land over the
//! route Ola publishes within one failed call, one backoff step, the
//! profile read and one call.
//!
//! Passes with and without fix (a) (seed 73, `wt/lfr2`): the failed call's
//! own refresh (`dm::send::call_refreshed`) already reads the published
//! route in the same attempt. Fix (a) changes only the attempt after a
//! refresh that still read the stale blob, and in the netsim the
//! accepter's profile watch (fix (b)) re-caches the new route before that
//! attempt; this scenario guards the stale-capture bound, not (a).

use std::time::Duration;

use rekindle_netsim::{latency, Fault};
use rekindle_outbox::{OutboxRetry, StreamKind};
use rekindle_types::dht_layout::profile::ROUTE_BLOB;

use crate::harness::{eventually, start};
use crate::pair::{accept, is_accepted_friend, key, request};
use crate::requester_route_bounds::{call_timeout, MARGIN};
use crate::requester_route_replaced::published_route;

/// A handshake row's first backoff step at its longest: its floor plus
/// the full first step (`rekindle_outbox` `StreamKind::policy`).
fn first_step() -> Duration {
    let OutboxRetry::Backoff { base, floor, .. } = StreamKind::FriendHandshake.policy();
    floor + base
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn an_accept_over_a_stale_captured_route_reads_the_published_one_next() {
    let (sim, mut nodes) = start(2, 73).await;
    for (node, name) in nodes.iter_mut().zip(["Tia", "Ola"]) {
        node.unlock_new_identity(name).await;
    }
    let tia = key(&nodes[0]);
    request(&sim, &nodes[1], &nodes[0]).await;
    let (profile, captured) = published_route(&sim, &nodes[1]).await;
    sim.fault(Fault::RouteDeath { node: 1 });
    let replaced = eventually(&sim, Duration::from_secs(60), || {
        std::future::ready(sim.network_get(&profile, ROUTE_BLOB).map(|v| v.data) != captured)
    })
    .await;
    assert!(replaced, "Ola published her replacement route");
    let asked = tokio::time::Instant::now();
    accept(&nodes[0], &nodes[1]).await;
    let accepted = eventually(&sim, Duration::from_secs(600), || {
        is_accepted_friend(&nodes[1], &tia)
    })
    .await;
    let took = asked.elapsed();
    assert!(accepted, "the accept reached Ola over her published route");
    let bound = call_timeout() + first_step() + latency::GET.max + call_timeout() + MARGIN;
    assert!(
        took <= bound,
        "the accept landed within one failed call, one backoff step and one call ({bound:?}): {took:?}"
    );
}
