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
use crate::pair::{befriend, holds_message, key, send, talk, unretired, LIMIT};

fn saw(events: &Events, peer: &str, status: &str) -> bool {
    events.any(|event| {
        matches!(event, SubscriptionEvent::Presence(PresenceEvent::FriendChanged {
            peer_key, snapshot, ..
        }) if peer_key == peer && snapshot.status.as_deref() == Some(status))
    })
}

/// Whether `node`'s own message `body` to `peer` is on its chain: sent or
/// further (a row counts as sent once its append lands).
async fn on_chain(node: &crate::harness::SimNode, peer: &str, body: &str) -> bool {
    use rekindle_types::message::SendState;
    own_state(node, peer, body, |s| {
        matches!(s, SendState::Sent | SendState::Delivered | SendState::Read)
    })
    .await
}

/// Whether `node`'s own message `body` to `peer` is in a state `ok` takes.
async fn own_state(
    node: &crate::harness::SimNode,
    peer: &str,
    body: &str,
    ok: impl Fn(rekindle_types::message::SendState) -> bool,
) -> bool {
    let history: Vec<rekindle_types::dto::dm::DmMessage> = serde_json::from_value(
        node.ok(IpcRequest::DmHistory {
            peer_key: peer.to_string(),
            limit: 50,
            before: None,
        })
        .await,
    )
    .expect("the history");
    history
        .iter()
        .any(|m| m.is_own && m.body == body && ok(m.send_state))
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
    // The hint leaves while the append is in flight (live A/B finding 42),
    // so `talk` returns before the entry lands and before the StreamRecord
    // naming the chain, a later row, reaches the reader: his watch on the
    // chain is held once a watched change of it reached him (F8).
    talk(&sim, &nodes[0], &nodes[1], "watched").await;
    assert!(
        eventually(&sim, LIMIT, || async { !unretired(&sim, 1).is_empty() }).await,
        "a change of the writer's chain reached the reader"
    );
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

/// Plan E2.6: Lev unlocks while his DHT is off, and the diff of Kim's next
/// hint reads nothing from the network. His records open from their local
/// copies and his watches are Veilid's desired state, placed once he is
/// online (a failed open is retried on the maintenance tick). With the two
/// then cut off from each other's messages (no hint or resend reaches him)
/// and his DHT back, his watch's catch-up reads the message he missed.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_watch_whose_open_failed_opens_on_a_later_tick_and_reads_what_it_missed() {
    // Every app message is dropped (no hint or resend reaches him).
    missed_while_shut(Fault::DropMessages { per_mille: 1000 }).await;
}

/// As above, with Kim and Lev cut off by a real `Partition` (plan E2.B S6,
/// finding F6): it also splits the DHT's replicas, each side reaching five
/// of ten (S4), and the replicas on Lev's side hold the message.
#[tokio::test(flavor = "current_thread", start_paused = true)]
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
    // The hint leaves while the append is in flight (live A/B finding 42),
    // so `talk` returns before the entry lands and before the StreamRecord
    // naming the chain, a later row, reaches the reader: his watch on the
    // chain is held once a watched change of it reached him (F8).
    talk(&sim, &nodes[0], &nodes[1], "watched").await;
    assert!(
        eventually(&sim, LIMIT, || async { !unretired(&sim, 1).is_empty() }).await,
        "a change of the writer's chain reached the reader"
    );
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
    // The message must be on Kim's chain before the cut (finding F6): her
    // append is a set in flight for up to a set's latency, and one that
    // lands after a `Partition` reaches only the five replicas on her side
    // (consensus there alone), so Lev's side would never hold it.
    assert!(
        eventually(&sim, LIMIT, || on_chain(&nodes[0], &lev, missed)).await,
        "Kim's message is on her chain"
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

/// Plan E2.6: Lev's DHT is off when Kim's first message announces her
/// participant record, a record he has never opened, so its open fails
/// (an unknown record offline is `TryAgain`) and no watch of it exists.
/// With every app message then dropped (no hint or resend reaches him),
/// Kim's next message is on her chain only; once his DHT is back, nothing
/// but the maintenance tick's retry places its watch (log: "a failed watch
/// opened on retry"), and that watch delivers her next message.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_participant_record_learned_offline_opens_on_a_later_tick() {
    use rekindle_types::message::SendState;
    let (sim, mut nodes) = start(2, 91).await;
    for (node, name) in nodes.iter_mut().zip(["Kim", "Lev"]) {
        node.unlock_new_identity(name).await;
    }
    befriend(&sim, &nodes[0], &nodes[1]).await;
    let (kim, lev) = (key(&nodes[0]), key(&nodes[1]));
    sim.fault(Fault::DhtOffline {
        node: 1,
        offline: true,
    });
    send(&nodes[0], &lev, "first").await;
    assert!(
        eventually(&sim, LIMIT, || own_state(&nodes[0], &lev, "first", |s| {
            matches!(s, SendState::Delivered | SendState::Read)
        }))
        .await,
        "Lev acknowledged the first message (her record announced before it)"
    );
    sim.fault(Fault::DropMessages { per_mille: 1000 });
    let missed = "while the record was not open";
    send(&nodes[0], &lev, missed).await;
    assert!(
        eventually(&sim, LIMIT, || on_chain(&nodes[0], &lev, missed)).await,
        "Kim's message is on her chain"
    );
    assert!(
        !holds_message(&nodes[1], &kim, missed).await,
        "nothing Lev reads reaches the network"
    );
    sim.fault(Fault::DhtOffline {
        node: 1,
        offline: false,
    });
    assert!(
        eventually(&sim, LIMIT, || holds_message(&nodes[1], &kim, missed)).await,
        "Lev's retried open watches the record and its diff reads the message"
    );
    // The read above may also come from the first hint's diff, whose open
    // the pool retried until the DHT was back; this one only the watch the
    // retry opened delivers.
    let watched = "by the retried watch";
    send(&nodes[0], &lev, watched).await;
    assert!(
        eventually(&sim, LIMIT, || holds_message(&nodes[1], &kim, watched)).await,
        "the retried watch delivers Kim's next message"
    );
}
