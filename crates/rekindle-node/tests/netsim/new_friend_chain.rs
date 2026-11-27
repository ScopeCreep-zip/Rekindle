//! A new friend's durable start (decision 67 item 2, F3): the accept
//! carries the accepter's presence key and its dm chain record, and it has a
//! durable copy in the requester's reply inbox (plan RV2). So with the pair's
//! calls stalled (`Fault::CallStalls`) and the accepter's app messages lost
//! on the way (`Fault::DropFrom`, the 2026-11-26 Linux run 2 class: every
//! question one way vanished; the DHT is a separate path), the requester
//! still reads the accept from its reply inbox, the accepter's presence
//! under the carried key, and the accepter's first DM from the chain it
//! watches. Before this, the chain key went only by a `StreamRecord`
//! `app_call` and the first DM after the friendship never landed.
//!
//! The request direction: the request carries the requester's chain
//! record, adopted by the accepter as it accepts, so whichever side writes
//! first after the friendship starts durably.

use std::time::Duration;

use rekindle_ipc::protocol::IpcRequest;
use rekindle_netsim::Fault;
use rekindle_types::presence::SessionStatus;
use rekindle_types::subscription_events::{PresenceEvent, SubscriptionEvent};

use crate::harness::{eventually, start, Events, SimNode, PASSPHRASE};
use crate::pair::{accept, holds_message, is_accepted_friend, key, request, send, LIMIT};

/// Longer than any assertion here waits.
const STALL: Duration = Duration::from_secs(3600);

fn saw(events: &Events, peer: &str, status: &str) -> bool {
    events.any(|event| {
        matches!(event, SubscriptionEvent::Presence(PresenceEvent::FriendChanged {
            peer_key, snapshot, ..
        }) if peer_key == peer && snapshot.status.as_deref() == Some(status))
    })
}

/// Ari (0) asks Bea (1); once Bea holds the request, every call between
/// them stalls and every app message `muted` sends is lost.
async fn asked_then_cut(seed: u64, muted: usize) -> (rekindle_netsim::Netsim, Vec<SimNode>) {
    let (sim, mut nodes) = start(2, seed).await;
    for (node, name) in nodes.iter_mut().zip(["Ari", "Bea"]) {
        node.unlock_new_identity(name).await;
    }
    request(&sim, &nodes[0], &nodes[1]).await;
    for node in [0, 1] {
        sim.fault(Fault::CallStalls {
            node,
            stall: Some(STALL),
        });
    }
    sim.fault(Fault::DropFrom {
        node: muted,
        per_mille: 1000,
    });
    (sim, nodes)
}

/// Bea, accepted at Ari, goes away and writes her first message; Ari sees
/// both.
async fn first_presence_and_dm(sim: &rekindle_netsim::Netsim, nodes: &[SimNode], events: &Events) {
    let bea = key(&nodes[1]);
    nodes[1]
        .ok(IpcRequest::PresenceSet {
            status: SessionStatus::Away,
        })
        .await;
    send(&nodes[1], &key(&nodes[0]), "first words").await;
    assert!(
        eventually(sim, LIMIT * 2, || async { saw(events, &bea, "away") }).await,
        "Ari opens Bea's status under the key her accept carried"
    );
    assert!(
        eventually(sim, LIMIT * 2, || holds_message(
            &nodes[0],
            &bea,
            "first words"
        ))
        .await,
        "Ari reads Bea's first message from the chain her accept named"
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn new_friend_presence_with_calls_stalled() {
    let (sim, nodes) = asked_then_cut(211, 1).await;
    let events = nodes[0].events().await;
    accept(&nodes[1], &nodes[0]).await;
    let bea = key(&nodes[1]);
    assert!(
        eventually(&sim, LIMIT * 2, || is_accepted_friend(&nodes[0], &bea)).await,
        "Ari reads the accept from his reply inbox"
    );
    first_presence_and_dm(&sim, &nodes, &events).await;
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn new_friend_presence_with_calls_stalled_accepter_restarts() {
    let (sim, nodes) = asked_then_cut(212, 1).await;
    let events = nodes[0].events().await;
    accept(&nodes[1], &nodes[0]).await;
    let bea = key(&nodes[1]);
    nodes[1].ok(IpcRequest::Lock).await;
    tokio::time::sleep(Duration::from_secs(20)).await;
    nodes[1]
        .ok(IpcRequest::Unlock {
            identity: bea.clone(),
            passphrase: PASSPHRASE.into(),
        })
        .await;
    assert!(
        eventually(&sim, LIMIT * 2, || is_accepted_friend(&nodes[0], &bea)).await,
        "Ari reads the accept Bea's restarted outbox finished"
    );
    first_presence_and_dm(&sim, &nodes, &events).await;
}

/// The request-direction twin: Ari writes the moment he holds Bea's
/// accept, with every call stalled and every app message of his lost; Bea
/// reads it from the chain his request named, which she adopted as she
/// accepted. Before, his chain key went only by a `StreamRecord` call.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn requester_first_dm_with_calls_stalled() {
    let (sim, nodes) = asked_then_cut(213, 0).await;
    accept(&nodes[1], &nodes[0]).await;
    let (ari, bea) = (key(&nodes[0]), key(&nodes[1]));
    assert!(
        eventually(&sim, LIMIT * 2, || is_accepted_friend(&nodes[0], &bea)).await,
        "Ari reads the accept from his reply inbox"
    );
    send(&nodes[0], &bea, "first words").await;
    assert!(
        eventually(&sim, LIMIT * 2, || holds_message(
            &nodes[1],
            &ari,
            "first words"
        ))
        .await,
        "Bea reads Ari's first message from the chain his request named"
    );
}
