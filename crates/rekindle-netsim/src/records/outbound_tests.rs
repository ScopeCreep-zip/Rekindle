//! Report marks, re-hints and the writer's own change (plan VU2 items 4
//! and 8), each against the Veilid 0.5.8 lines [`super::outbound`] and
//! [`super::watches`] cite.

use std::sync::Arc;
use std::time::Duration;

use rekindle_records::port::{DhtBackend, PortEvent, RecordSchema, ValueChange};

use super::dht::SimDht;
use crate::{Netsim, Node};

fn dht(sim: &Netsim, node: &Node) -> SimDht {
    SimDht::new(node.index, Arc::clone(&sim.net), Arc::clone(&node.rng))
}

/// One watch tick and a message latency of deliveries; what node `i` heard.
async fn tick(sim: &Netsim, nodes: &mut [Node], i: usize) -> Vec<ValueChange> {
    sim.net.inner.lock().next_tick = Some(tokio::time::Instant::now());
    sim.net.deliver();
    tokio::time::advance(Duration::from_secs(1)).await;
    sim.net.deliver();
    let mut heard = Vec::new();
    while let Ok(event) = nodes[i].events.try_recv() {
        if let PortEvent::ValueChange(change) = event {
            heard.push(change);
        }
    }
    heard
}

/// A record node 0 seeded at subkeys 0..4, read in full and watched by
/// node 1, its establishing inspection run.
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
    b.open(&key, None, rekindle_records::port::OpenIntent::Hold)
        .await
        .unwrap();
    for subkey in 0..4 {
        b.get(&key, subkey, true).await.unwrap();
    }
    assert!(b.watch(&key, None, None, None).await.unwrap());
    for _ in 0..8 {
        assert!(tick(sim, nodes, 1).await.is_empty());
    }
    (key, a, b)
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_range_hint_on_an_unread_subkey_waits_for_a_network_read() {
    let (sim, mut nodes) = Netsim::nodes(2, 41);
    let (key, a, b) = watched(&sim, &mut nodes).await;
    // One subkey: reported with its value, which marks it.
    a.set(&key, 3, vec![1], None).await.unwrap();
    let heard = tick(&sim, &mut nodes, 1).await;
    assert_eq!(heard.len(), 1, "{heard:?}");
    assert_eq!(
        (heard[0].subkeys.clone(), heard[0].value.is_some()),
        (vec![3], true)
    );
    // 2 and 3 in one push: the range, 3 suppressed (`outbound_watch_state.rs:251-266`).
    a.set(&key, 2, vec![2], None).await.unwrap();
    a.set(&key, 3, vec![2], None).await.unwrap();
    let heard = tick(&sim, &mut nodes, 1).await;
    assert_eq!(heard.len(), 1, "{heard:?}");
    assert_eq!(
        (heard[0].subkeys.clone(), heard[0].value.clone()),
        (vec![2], None)
    );
    // The scenario-facing tallies saw both: a coalesced push, and 3 left
    // out of its report (every holding replica pushed, so at least one).
    assert!(sim.coalesced_pushes(1, &key) >= 1);
    assert!(sim.suppressed_hints(1, &key) >= 1);
    assert_eq!(sim.unread_marks(1, &key), vec![2, 3]);
    // A local read leaves the mark: nothing owed, a later range still drops 3.
    b.get(&key, 3, false).await.unwrap();
    assert!(tick(&sim, &mut nodes, 1).await.is_empty());
    // A network read of 3 consumes it and owes one re-hint of 3
    // (`watch_value.rs:2275-2320`), then 3 is news again.
    b.get(&key, 3, true).await.unwrap();
    let heard = tick(&sim, &mut nodes, 1).await;
    assert_eq!(heard.len(), 1, "{heard:?}");
    assert_eq!(
        (heard[0].subkeys.clone(), heard[0].value.clone()),
        (vec![3], None)
    );
    // Reading 2 owes its own re-hint: the push came from every holding
    // replica, so copies of 2 were suppressed too.
    b.get(&key, 3, true).await.unwrap();
    b.get(&key, 2, true).await.unwrap();
    let heard = tick(&sim, &mut nodes, 1).await;
    assert_eq!(heard.len(), 1, "{heard:?}");
    assert_eq!(heard[0].subkeys, vec![2]);
    b.get(&key, 2, true).await.unwrap();
    assert!(tick(&sim, &mut nodes, 1).await.is_empty());
    a.set(&key, 2, vec![3], None).await.unwrap();
    a.set(&key, 3, vec![3], None).await.unwrap();
    let heard = tick(&sim, &mut nodes, 1).await;
    assert_eq!(heard.len(), 1, "{heard:?}");
    assert_eq!(heard[0].subkeys, vec![2, 3]);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn the_writer_hears_its_own_change() {
    let (sim, mut nodes) = Netsim::nodes(2, 42);
    let a = dht(&sim, &nodes[0]);
    let key = a
        .create(&RecordSchema::Dflt { owner_subkeys: 1 }, None)
        .await
        .unwrap()
        .key;
    a.set(&key, 0, vec![0], None).await.unwrap();
    assert!(a.watch(&key, None, None, None).await.unwrap());
    for _ in 0..8 {
        tick(&sim, &mut nodes, 0).await;
    }
    a.set(&key, 0, vec![1], None).await.unwrap();
    // The push races the set's return: the writer's copy is not yet newer
    // (Veilid's set returns at its span's end, the replica sends within
    // its 1 s tick).
    let ours = sim.net.inner.lock().nodes[0]
        .local
        .remove(&(key.clone(), 0));
    assert_eq!(ours.map(|v| v.data), Some(vec![1]));
    let heard = tick(&sim, &mut nodes, 0).await;
    assert_eq!(heard.len(), 1, "{heard:?}");
    assert_eq!(
        heard[0].value.as_ref().map(|v| v.data.clone()),
        Some(vec![1])
    );
}
