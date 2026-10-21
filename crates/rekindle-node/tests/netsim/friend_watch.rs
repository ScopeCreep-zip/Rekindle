//! Friend watches (plan E2.5, D23): no poll reads a friend's profile. Their
//! status change arrives by the profile record's watch, and after that
//! watch dies (Veilid's last change, `count == 0`) the pool re-arms it, so
//! the next change arrives too.

use std::time::Duration;

use rekindle_ipc::protocol::IpcRequest;
use rekindle_netsim::Fault;
use rekindle_types::presence::SessionStatus;
use rekindle_types::subscription_events::{PresenceEvent, SubscriptionEvent};

use crate::harness::{eventually, start, Events, PASSPHRASE};
use crate::pair::{befriend, holds_message, key, send, talk, LIMIT};

fn saw(events: &Events, peer: &str, status: &str) -> bool {
    events.any(|event| {
        matches!(event, SubscriptionEvent::Presence(PresenceEvent::FriendChanged {
            peer_key, snapshot, ..
        }) if peer_key == peer && snapshot.status.as_deref() == Some(status))
    })
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_status_change_arrives_by_watch_and_survives_a_watch_death() {
    let (sim, mut nodes) = start(2, 83).await;
    for (node, name) in nodes.iter_mut().zip(["Kim", "Lev"]) {
        node.unlock_new_identity(name).await;
    }
    befriend(&sim, &nodes[0], &nodes[1]).await;
    let kim = key(&nodes[0]);
    let events = nodes[1].events().await;

    nodes[0]
        .ok(IpcRequest::PresenceSet {
            status: SessionStatus::Away,
        })
        .await;
    assert!(
        eventually(&sim, LIMIT, || async { saw(&events, &kim, "away") }).await,
        "Lev sees Kim away by the watch"
    );

    let code: rekindle_types::dto::friends::FriendCodeView =
        serde_json::from_value(nodes[0].ok(IpcRequest::FriendCode).await).expect("the code");
    sim.fault(Fault::WatchDeath {
        node: 1,
        key: code.profile_key,
    });
    nodes[0]
        .ok(IpcRequest::PresenceSet {
            status: SessionStatus::Busy,
        })
        .await;
    assert!(
        eventually(&sim, LIMIT, || async { saw(&events, &kim, "busy") }).await,
        "Lev sees Kim busy after the watch was re-armed"
    );
}

/// A status change made while Lev is detached fires no watch of his; his
/// reattach (the network-ready trigger) re-opens the watches and applies
/// what changed meanwhile.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_change_missed_while_detached_is_recovered_on_reattach() {
    let (sim, mut nodes) = start(2, 89).await;
    for (node, name) in nodes.iter_mut().zip(["Mia", "Ned"]) {
        node.unlock_new_identity(name).await;
    }
    befriend(&sim, &nodes[0], &nodes[1]).await;
    let mia = key(&nodes[0]);
    let events = nodes[1].events().await;

    sim.fault(Fault::Detach {
        node: 1,
        detached: true,
    });
    nodes[0]
        .ok(IpcRequest::PresenceSet {
            status: SessionStatus::Busy,
        })
        .await;
    assert!(
        !eventually(&sim, LIMIT, || async { saw(&events, &mia, "busy") }).await,
        "no watch change reaches a detached node"
    );
    sim.fault(Fault::Detach {
        node: 1,
        detached: false,
    });
    assert!(
        eventually(&sim, LIMIT, || async { saw(&events, &mia, "busy") }).await,
        "Ned sees Mia busy after his reattach"
    );
}

/// A participant record received mid-session is watched at once: with
/// every app message dropped (no hint arrives), a message still reaches
/// the peer through the watch on the sender's chain.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_participant_record_received_mid_session_is_watched() {
    let (sim, mut nodes) = start(2, 97).await;
    for (node, name) in nodes.iter_mut().zip(["Ora", "Pim"]) {
        node.unlock_new_identity(name).await;
    }
    befriend(&sim, &nodes[0], &nodes[1]).await;
    crate::pair::talk(&sim, &nodes[0], &nodes[1], "hello").await;
    let (ora, pim) = (key(&nodes[0]), key(&nodes[1]));

    sim.fault(Fault::DropMessages { per_mille: 1000 });
    crate::pair::send(&nodes[0], &pim, "by the watch").await;
    assert!(
        eventually(&sim, LIMIT, || crate::pair::holds_message(
            &nodes[1],
            &ora,
            "by the watch"
        ))
        .await,
        "Pim reads Ora's chain by its watch"
    );
}

/// Plan E2.6: Lev unlocks while his DHT is off, so every watch his unlock
/// opens fails, and so does the diff of Kim's next hint. With the two then
/// cut off from each other's messages (no hint or resend reaches him) and his DHT back, no network
/// change re-opens them. The maintenance tick's retry does, and the diff it
/// runs reads the message he missed.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_watch_whose_open_failed_opens_on_a_later_tick_and_reads_what_it_missed() {
    // Every app message is dropped (no hint or resend reaches him).
    missed_while_shut(Fault::DropMessages { per_mille: 1000 }).await;
}

/// As above, with Kim and Lev cut off by a real `Partition` (plan E2.B S6,
/// finding F6): it also splits the DHT's replicas, each side reaching five
/// of ten (S4), and the replicas on Lev's side hold the message.
#[tokio::test(flavor = "current_thread", start_paused = true)]
#[ignore = "E2.B finding F6: under a Partition Lev never reads the missed message (watch catch-up modeled, S6); host/adapters/friends/watch.rs retry_failed_watches never logs `a failed watch opened on retry`: suspect participant_records/the wanted check settling the retry, or watch_participant staying pending"]
async fn a_watch_whose_open_failed_reads_what_it_missed_across_a_partition() {
    missed_while_shut(Fault::Partition {
        a: vec![0],
        b: vec![1],
    })
    .await;
}

/// Lev unlocks with his DHT off and misses Kim's message; then `cut` splits
/// them and his DHT returns: his retried watch reads the message.
async fn missed_while_shut(cut: Fault) {
    let (sim, mut nodes) = start(2, 87).await;
    for (node, name) in nodes.iter_mut().zip(["Kim", "Lev"]) {
        node.unlock_new_identity(name).await;
    }
    befriend(&sim, &nodes[0], &nodes[1]).await;
    talk(&sim, &nodes[0], &nodes[1], "hello").await;
    let (kim, lev) = (key(&nodes[0]), key(&nodes[1]));
    nodes[1].ok(IpcRequest::Lock).await;
    sim.fault(Fault::DhtOffline {
        node: 1,
        offline: true,
    });
    nodes[1]
        .ok(IpcRequest::Unlock {
            identity: lev.clone(),
            passphrase: PASSPHRASE.into(),
        })
        .await;
    let missed = "while your watches were shut";
    send(&nodes[0], &lev, missed).await;
    assert!(
        !eventually(&sim, Duration::from_secs(20), || holds_message(
            &nodes[1], &kim, missed
        ))
        .await,
        "nothing Lev reads reaches the network"
    );
    sim.fault(cut);
    sim.fault(Fault::DhtOffline {
        node: 1,
        offline: false,
    });
    assert!(
        eventually(&sim, LIMIT, || holds_message(&nodes[1], &kim, missed)).await,
        "Lev's retried watch opens and its diff reads the message"
    );
}
