//! Live finding L-FR2 (Linux, 30 friendships, run 8): the requester
//! replaced its route seconds around its request, and the accepter's
//! `FriendAccept` call kept timing out for 7+ minutes on the route captured
//! from the request, while the requester's first DM, typed at once, was
//! never sent. Ola asks Tia; right after, the route her request named
//! goes silent ([`Fault::RouteSilentDeath`]); Ola finds it by her own
//! self-test and publishes a tested replacement. Tia accepts at once and
//! Ola writes to Tia at once, before any accept reached her. Tia's accept
//! must land over Ola's current route within a bounded time, and Ola's
//! first DM, queued while she waits, must reach Tia once the session is
//! open.
//!
//! The bound (`requester_route_bounds`): with a silent death Ola learns
//! of it only by her own self-test, so the accept may take the worst
//! detection + the republish + the accepter's watch and one call + a
//! margin (~171 s), and her first DM one call after that. Both are wide;
//! what fix (b) owns is the step after the republish: Tia's waiting row
//! goes over the new route as soon as her watch hears it, not at its next
//! backoff. Seed 71: republished ~97 s, accept 100.8 s, DM 101.4 s; with
//! the profile watch's forward move removed the row waits out its backoff
//! and lands at 119.8 s, ~22 s after the republish (fails the step).

use std::time::Duration;

use rekindle_ipc::protocol::IpcRequest;
use rekindle_netsim::{Fault, Netsim};
use rekindle_records::port::RouteClass;
use rekindle_types::dht_layout::profile::ROUTE_BLOB;

use crate::harness::{eventually, start, SimNode};
use crate::pair::{accept, holds_message, is_accepted_friend, key, request, send};
use crate::requester_route_bounds::{
    after_republish, call_timeout, republish, worst_detection, MARGIN,
};

const FIRST: &str = "hi, right after my request";

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn an_accept_lands_over_the_requesters_new_route_and_her_first_dm_follows() {
    let (sim, mut nodes) = start(2, 71).await;
    for (node, name) in nodes.iter_mut().zip(["Tia", "Ola"]) {
        node.unlock_new_identity(name).await;
    }
    let (tia, ola) = (key(&nodes[0]), key(&nodes[1]));
    request(&sim, &nodes[1], &nodes[0]).await;
    // Right after her request landed, the route it named stops delivering
    // and no one is told (the live case: Veilid reports nothing; Ola finds
    // it by inbound silence while she awaits an answer, rotates on that evidence
    // and republishes a tested replacement, plan RV1).
    sim.fault(Fault::RouteSilentDeath {
        node: 1,
        class: RouteClass::General,
    });
    let asked = tokio::time::Instant::now();
    let (profile, captured) = published_route(&sim, &nodes[1]).await;
    accept(&nodes[0], &nodes[1]).await;
    send(&nodes[1], &tia, FIRST).await;
    let replaced = eventually(&sim, Duration::from_secs(600), || {
        std::future::ready(sim.network_get(&profile, ROUTE_BLOB).map(|v| v.data) != captured)
    })
    .await;
    assert!(replaced, "Ola republished a tested replacement");
    let republished = asked.elapsed();
    assert!(
        republished <= worst_detection() + republish(),
        "the replacement was on the network within its evidence's worst case: {republished:?}"
    );
    let accepted = eventually(&sim, Duration::from_secs(600), || {
        is_accepted_friend(&nodes[1], &tia)
    })
    .await;
    let took = asked.elapsed();
    assert!(accepted, "the accept reached Ola over her new route");
    let accept_bound = worst_detection() + republish() + after_republish() + MARGIN;
    assert!(
        took <= accept_bound,
        "the accept landed within {accept_bound:?}: {took:?}"
    );
    let step = after_republish() + MARGIN;
    assert!(
        took.saturating_sub(republished) <= step,
        "the accept followed the republish within {step:?} (fix (b)): {:?}",
        took.saturating_sub(republished)
    );
    let delivered = eventually(&sim, Duration::from_secs(600), || {
        holds_message(&nodes[0], &ola, FIRST)
    })
    .await;
    assert!(
        delivered,
        "Ola's first DM reached Tia once the session was open"
    );
    let dm_bound = took + call_timeout() + MARGIN;
    assert!(
        asked.elapsed() <= dm_bound,
        "the first DM one send after the accept, within {dm_bound:?}: {:?}",
        asked.elapsed()
    );
}

/// `node`'s profile record key and the route blob it holds on the network.
pub async fn published_route(sim: &Netsim, node: &SimNode) -> (String, Option<Vec<u8>>) {
    let shown = node.ok(IpcRequest::IdentityShow).await;
    let profile = shown["profileDhtKey"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    assert!(!profile.is_empty(), "a profile record");
    let blob = sim.network_get(&profile, ROUTE_BLOB).map(|v| v.data);
    (profile, blob)
}
