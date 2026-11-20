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
//! FAILS at `50295a012` (slice owed, L-FR2): the accept lands after 109 s
//! (seed 71) against a 60 s bound; which wait dominates is not traced yet.

use std::time::Duration;

use rekindle_netsim::Fault;
use rekindle_records::port::RouteClass;

use crate::harness::{eventually, start};
use crate::pair::{accept, holds_message, is_accepted_friend, key, request, send};

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
    // it by her own self-test and republishes a tested replacement).
    sim.fault(Fault::RouteSilentDeath {
        node: 1,
        class: RouteClass::General,
    });
    let asked = tokio::time::Instant::now();
    accept(&nodes[0], &nodes[1]).await;
    send(&nodes[1], &tia, FIRST).await;
    let accepted = eventually(&sim, Duration::from_secs(600), || {
        is_accepted_friend(&nodes[1], &tia)
    })
    .await;
    let took = asked.elapsed();
    assert!(accepted, "the accept reached Ola over her new route");
    assert!(
        took <= ACCEPT_BOUND,
        "the accept landed within {ACCEPT_BOUND:?}: {took:?}"
    );
    let delivered = eventually(&sim, Duration::from_secs(600), || {
        holds_message(&nodes[0], &ola, FIRST)
    })
    .await;
    assert!(
        delivered,
        "Ola's first DM reached Tia once the session was open"
    );
    assert!(
        asked.elapsed() <= DM_BOUND,
        "the first DM within {DM_BOUND:?}: {:?}",
        asked.elapsed()
    );
}

const ACCEPT_BOUND: Duration = Duration::from_secs(60);
const DM_BOUND: Duration = Duration::from_secs(90);
