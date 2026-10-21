//! The catch-up (M2) and offline open/get (M3) rules (plan E2.B S6), each
//! against the Veilid lines [`super::catchup`] and [`super::dht`] cite.

use std::sync::Arc;
use std::time::Duration;

use rekindle_records::port::{DhtBackend, DhtErrorKind, PortEvent, RecordSchema, ValueChange};

use super::dht::SimDht;
use crate::{Fault, Netsim, Node};

fn dht(sim: &Netsim, node: &Node) -> SimDht {
    SimDht::new(node.index, Arc::clone(&sim.net), Arc::clone(&node.rng))
}

/// A record node 0 created and seeded at subkeys 0..4, read in full and
/// watched by node 1 (so its establishing inspection finds nothing).
async fn watched(sim: &Netsim, nodes: &mut [Node]) -> (String, SimDht, SimDht) {
    let (a, b) = (dht(sim, &nodes[0]), dht(sim, &nodes[1]));
    let key = a
        .create(&RecordSchema::Dflt { owner_subkeys: 4 }, None)
        .await
        .unwrap()
        .key;
    for subkey in 0..4 {
        a.set(&key, subkey, vec![0], None).await.unwrap();
    }
    b.open(&key, None).await.unwrap();
    for subkey in 0..4 {
        b.get(&key, subkey, true).await.unwrap();
    }
    assert!(b.watch(&key, None, None, None).await.unwrap());
    assert!(heard(sim, nodes, 1).await.is_empty());
    (key, a, b)
}

/// Run a watch tick, wait out an inspection's latency, deliver; the value
/// changes node `i` heard.
async fn heard(sim: &Netsim, nodes: &mut [Node], i: usize) -> Vec<ValueChange> {
    sim.net.inner.lock().next_tick = Some(tokio::time::Instant::now());
    sim.net.deliver();
    tokio::time::advance(Duration::from_secs(7)).await;
    sim.net.deliver();
    let mut changes = Vec::new();
    while let Ok(event) = nodes[i].events.try_recv() {
        if let PortEvent::ValueChange(change) = event {
            changes.push(change);
        }
    }
    changes
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_reattached_node_hears_the_changes_it_missed() {
    let (sim, mut nodes) = Netsim::nodes(2, 21);
    let (key, a, b) = watched(&sim, &mut nodes).await;
    sim.fault(Fault::Detach {
        node: 1,
        detached: true,
    });
    a.set(&key, 1, vec![1], None).await.unwrap();
    a.set(&key, 2, vec![2], None).await.unwrap();
    assert!(heard(&sim, &mut nodes, 1).await.is_empty());
    sim.fault(Fault::Detach {
        node: 1,
        detached: false,
    });
    let changes = heard(&sim, &mut nodes, 1).await;
    // The range of newer subkeys, the first one's value, stored before.
    assert_eq!(changes.len(), 1, "{changes:?}");
    assert_eq!(changes[0].subkeys, vec![1, 2]);
    let value = changes[0].value.clone().expect("the first subkey's value");
    assert_eq!((value.seq, value.data), (1, vec![1]));
    assert_eq!(b.get(&key, 1, false).await.unwrap().unwrap().seq, 1);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn dht_back_online_with_nothing_newer_reports_nothing() {
    let (sim, mut nodes) = Netsim::nodes(2, 22);
    let (_key, _a, _b) = watched(&sim, &mut nodes).await;
    for offline in [true, false] {
        sim.fault(Fault::DhtOffline { node: 1, offline });
        assert!(heard(&sim, &mut nodes, 1).await.is_empty());
    }
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn heal_is_a_readiness_return_and_an_offline_inspection_retries() {
    let (sim, mut nodes) = Netsim::nodes(2, 23);
    let (key, a, _b) = watched(&sim, &mut nodes).await;
    sim.fault(Fault::DhtOffline {
        node: 1,
        offline: true,
    });
    a.set(&key, 3, vec![3], None).await.unwrap();
    sim.fault(Fault::Heal);
    let changes = heard(&sim, &mut nodes, 1).await;
    assert_eq!(changes.len(), 1, "{changes:?}");
    assert_eq!(changes[0].subkeys, vec![3]);
    // A queued inspection that finds the node offline waits 5 s and runs.
    super::catchup::enqueue(&mut sim.net.inner.lock(), 1, &key, sim.net.now_us());
    sim.net.inner.lock().faults.dht_offline.insert(1);
    sim.net.inner.lock().next_tick = Some(tokio::time::Instant::now());
    sim.net.deliver();
    sim.net.inner.lock().nodes[1].was_online = true;
    sim.net.inner.lock().faults.dht_offline.remove(&1);
    let queued = sim.net.inner.lock().nodes[1].inspects.get(&key).copied();
    assert!(queued.is_some_and(|at| at > sim.net.now_us()), "{queued:?}");
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_tick_lag_inspects_every_watch_at_most_once_per_interval() {
    let (sim, mut nodes) = Netsim::nodes(2, 24);
    let (key, a, _b) = watched(&sim, &mut nodes).await;
    // A change the watch never heard of (the local copy is behind).
    let behind = |sim: &Netsim| {
        sim.net.inner.lock().nodes[1]
            .local
            .remove(&(key.clone(), 0));
    };
    behind(&sim);
    sim.fault(Fault::TickLag { node: 1, ms: 1_000 });
    assert!(heard(&sim, &mut nodes, 1).await.is_empty());
    sim.fault(Fault::TickLag { node: 1, ms: 3_000 });
    let changes = heard(&sim, &mut nodes, 1).await;
    assert_eq!(changes.len(), 1, "{changes:?}");
    assert_eq!(changes[0].subkeys, vec![0]);
    // Within 30 s a second lag inspects nothing.
    behind(&sim);
    sim.fault(Fault::TickLag { node: 1, ms: 3_000 });
    assert!(sim.net.inner.lock().nodes[1].inspects.is_empty());
    drop(a);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_known_record_opens_and_reads_offline() {
    let (sim, mut nodes) = Netsim::nodes(2, 25);
    let (key, a, b) = watched(&sim, &mut nodes).await;
    b.close(&key).await.unwrap();
    let unknown = a
        .create(&RecordSchema::Dflt { owner_subkeys: 1 }, None)
        .await
        .unwrap()
        .key;
    a.set(&unknown, 0, vec![1], None).await.unwrap();
    sim.fault(Fault::DhtOffline {
        node: 1,
        offline: true,
    });
    // A record in the local store opens with no network (`open_record.rs:27-40`).
    b.open(&key, None).await.unwrap();
    // A local get answers offline; a forced one is `TryAgain` (`get_value.rs:78-92`).
    assert_eq!(b.get(&key, 2, false).await.unwrap().unwrap().data, vec![0]);
    let forced = b.get(&key, 2, true).await.unwrap_err();
    assert_eq!(forced.kind, DhtErrorKind::TryAgain);
    // An unknown record needs the network (`open_record.rs:41-44`).
    let open = b.open(&unknown, None).await.unwrap_err();
    assert_eq!(open.kind, DhtErrorKind::TryAgain);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_local_get_with_no_copy_is_try_again_offline() {
    let (sim, nodes) = Netsim::nodes(2, 26);
    let (a, b) = (dht(&sim, &nodes[0]), dht(&sim, &nodes[1]));
    let key = a
        .create(&RecordSchema::Dflt { owner_subkeys: 2 }, None)
        .await
        .unwrap()
        .key;
    a.set(&key, 0, vec![1], None).await.unwrap();
    b.open(&key, None).await.unwrap();
    sim.fault(Fault::DhtOffline {
        node: 1,
        offline: true,
    });
    let err = b.get(&key, 0, false).await.unwrap_err();
    assert_eq!(err.kind, DhtErrorKind::TryAgain);
}
