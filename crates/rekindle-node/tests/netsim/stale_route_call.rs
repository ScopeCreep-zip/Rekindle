//! A first-contact call over a route that died after we imported it (live
//! A/B 2026-10-23, where a call over the route a peer had before its
//! restart took 2 min 16 s of attempts to land). Ola's request reaches Tia
//! through the invite's inbox, so Tia imports and holds Ola's route from it;
//! then Ola's routes die, which tells only Ola (an importer is never told,
//! `routes`), and she allocates and publishes a new one. Tia's accept is a
//! receipted call: it goes over the dead route she holds, which Veilid
//! still resolves from her cache and sends into, so no answer comes back
//! and the call times out (`rekindle-netsim` `messages.rs` `call_over`).
//! It then calls again over the one Ola's profile holds
//! (`rekindle-transport` `dm::send::call_datagram_answer`'s
//! `call_refreshed`), so it lands within that one attempt, not after the
//! row's retry.
//!
//! Fails with the timed-out call answered as an error instead of calling
//! again (`call_datagram_answer`'s `Err(e)` arm): the accept lands only on
//! the row's retry, past [`ONE_ATTEMPT`].

use std::time::Duration;

use rekindle_ipc::protocol::IpcRequest;
use rekindle_netsim::Fault;

use crate::harness::{eventually, start};
use crate::pair::{accept, has_request_from, is_accepted_friend, key};

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_first_contact_over_a_dead_route_is_called_again_over_the_profiles() {
    let (sim, mut nodes) = start(2, 67).await;
    for (node, name) in nodes.iter_mut().zip(["Tia", "Ola"]) {
        node.unlock_new_identity(name).await;
    }
    let invite: rekindle_types::dto::friends::InviteCreated =
        serde_json::from_value(nodes[0].ok(IpcRequest::FriendInviteCreate).await)
            .expect("the invite");
    nodes[1]
        .ok(IpcRequest::FriendAddFromInvite { invite: invite.url })
        .await;
    let ola = key(&nodes[1]);
    assert!(
        eventually(&sim, Duration::from_secs(60), || has_request_from(
            &nodes[0], &ola
        ))
        .await,
        "Tia holds Ola's request and the route it named"
    );
    // Ola's routes die; only she hears it. She allocates and publishes a
    // new one.
    sim.fault(Fault::RouteDeath { node: 1 });
    tokio::time::sleep(Duration::from_secs(60)).await;
    let asked = tokio::time::Instant::now();
    accept(&nodes[0], &nodes[1]).await;
    let tia = key(&nodes[0]);
    let accepted = eventually(&sim, Duration::from_secs(300), || {
        is_accepted_friend(&nodes[1], &tia)
    })
    .await;
    let took = asked.elapsed();
    assert!(accepted, "the accept reached Ola over her new route");
    // One attempt: the call over the dead route waits out its timeout, then
    // the profile read and the call over the new route; the row's retry
    // would come only after its backoff.
    assert!(
        took <= ONE_ATTEMPT,
        "the accept landed within one attempt: {took:?}"
    );
}

/// One attempt's bound. With this seed on the Veilid 0.5.8 model the
/// accept lands 15.5 s after it is asked (the dead route's call timeout,
/// the profile read, the call over the new route, and Ola applying it);
/// without the refresh the row fails that attempt and lands on its retry,
/// 21.9 s.
const ONE_ATTEMPT: Duration = Duration::from_secs(18);
