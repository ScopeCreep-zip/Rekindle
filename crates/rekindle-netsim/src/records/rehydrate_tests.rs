//! Rehydration and store eviction (M5, plan E2.B S7), each against the
//! Veilid lines [`super::rehydrate`] and [`super::stores`] cite.

use std::collections::BTreeSet;
use std::sync::Arc;

use rekindle_records::port::{DhtBackend, DhtErrorKind, RecordSchema};

use super::dht::SimDht;
use super::rehydrate::pending;
use super::stores::REMOTE_MAX_RECORDS;
use crate::{Fault, Netsim, Node};

fn dht(sim: &Netsim, node: &Node) -> SimDht {
    SimDht::new(node.index, Arc::clone(&sim.net), Arc::clone(&node.rng))
}

/// Run one storage-manager tick now.
fn tick(sim: &Netsim) {
    sim.net.inner.lock().next_tick = Some(tokio::time::Instant::now());
    sim.net.deliver();
}

/// How many of `key`'s replicas hold a value at `subkey`.
fn copies(sim: &Netsim, key: &str, subkey: usize) -> usize {
    sim.net.inner.lock().records[key]
        .replicas
        .iter()
        .filter(|r| r.values[subkey].is_some())
        .count()
}

/// The replicas lose every value of `key` (as on eviction everywhere).
fn wipe(sim: &Netsim, key: &str) {
    for replica in &mut sim.net.inner.lock().records.get_mut(key).unwrap().replicas {
        replica.values.iter_mut().for_each(|v| *v = None);
    }
}

async fn seeded(a: &SimDht) -> String {
    let key = a
        .create(&RecordSchema::Dflt { owner_subkeys: 2 }, None)
        .await
        .unwrap()
        .key;
    a.set(&key, 0, vec![7], None).await.unwrap();
    key
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn an_open_of_a_held_record_pushes_what_the_replicas_lost() {
    let (sim, nodes) = Netsim::nodes(2, 31);
    let a = dht(&sim, &nodes[0]);
    let key = seeded(&a).await;
    wipe(&sim, &key);
    a.open(&key, None).await.unwrap();
    // The first tick finds what to push, the next pushes it.
    tick(&sim);
    assert_eq!(copies(&sim, &key, 0), 0);
    tick(&sim);
    assert!(copies(&sim, &key, 0) >= 5);
    assert_eq!(copies(&sim, &key, 1), 0, "nothing local, nothing pushed");
    let inner = sim.net.inner.lock();
    assert!(inner.nodes[0].pushes.is_empty() && inner.nodes[0].rehydrations.is_empty());
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_set_refused_offline_queues_nothing() {
    let (sim, nodes) = Netsim::nodes(2, 32);
    let a = dht(&sim, &nodes[0]);
    let key = seeded(&a).await;
    sim.fault(Fault::DhtOffline {
        node: 0,
        offline: true,
    });
    let refused = a.set(&key, 1, vec![9], None).await.unwrap_err();
    assert_eq!(refused.kind, DhtErrorKind::TryAgain);
    sim.fault(Fault::DhtOffline {
        node: 0,
        offline: false,
    });
    a.open(&key, None).await.unwrap();
    tick(&sim);
    tick(&sim);
    assert_eq!(copies(&sim, &key, 1), 0);
    assert!(a.get(&key, 1, false).await.unwrap().is_none());
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn an_offline_open_rehydrates_once_back_online() {
    let (sim, nodes) = Netsim::nodes(2, 33);
    let a = dht(&sim, &nodes[0]);
    let key = seeded(&a).await;
    wipe(&sim, &key);
    sim.fault(Fault::DhtOffline {
        node: 0,
        offline: true,
    });
    a.open(&key, None).await.unwrap();
    tick(&sim);
    tick(&sim);
    assert_eq!(copies(&sim, &key, 0), 0);
    assert!(!pending(&sim.net.inner.lock(), 0), "offline: no work");
    assert!(sim.net.inner.lock().nodes[0].rehydrations.contains(&key));
    sim.fault(Fault::Heal);
    tick(&sim);
    tick(&sim);
    assert!(copies(&sim, &key, 0) >= 5);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_push_short_of_consensus_stays_queued_until_the_cut_heals() {
    let (sim, nodes) = Netsim::nodes(10, 34);
    let a = dht(&sim, &nodes[0]);
    let key = seeded(&a).await;
    wipe(&sim, &key);
    sim.fault(Fault::Partition {
        a: vec![0],
        b: (1..10).collect(),
    });
    a.open(&key, None).await.unwrap();
    tick(&sim);
    tick(&sim);
    assert!(copies(&sim, &key, 0) < 3);
    assert!(sim.net.inner.lock().nodes[0]
        .pushes
        .contains_key(&(key.clone(), 0)));
    assert!(!pending(&sim.net.inner.lock(), 0), "the same fanout again");
    sim.fault(Fault::Heal);
    assert!(pending(&sim.net.inner.lock(), 0));
    tick(&sim);
    assert!(copies(&sim, &key, 0) >= 5);
    assert!(sim.net.inner.lock().nodes[0].pushes.is_empty());
}

/// The stores holding `key` at subkey 0.
fn stores_of(sim: &Netsim, key: &str) -> BTreeSet<usize> {
    sim.net.inner.lock().records[key]
        .replicas
        .iter()
        .filter(|r| r.values[0].is_some())
        .map(|r| r.store)
        .collect()
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_full_store_evicts_its_least_recently_used_record() {
    let (sim, nodes) = Netsim::nodes(1, 35);
    let a = dht(&sim, &nodes[0]);
    let mut keys = vec![seeded(&a).await];
    // Create records until a store is full; a read uses the first record
    // after every create, so each store's LRU is its oldest other record.
    let (evicted, store) = loop {
        assert!(keys.len() < 10 * REMOTE_MAX_RECORDS, "no store filled");
        let before: Vec<BTreeSet<usize>> = keys.iter().map(|k| stores_of(&sim, k)).collect();
        keys.push(seeded(&a).await);
        a.get(&keys[0], 0, true).await.unwrap();
        let lost: Vec<(usize, usize)> = before
            .iter()
            .enumerate()
            .flat_map(|(i, held)| {
                let now = stores_of(&sim, &keys[i]);
                held.difference(&now)
                    .map(move |s| (i, *s))
                    .collect::<Vec<_>>()
            })
            .collect();
        if let Some(first) = lost.first() {
            for (i, s) in &lost {
                let oldest = (1..before.len()).find(|j| before[*j].contains(s));
                assert_eq!(Some(*i), oldest, "store {s} evicted its LRU");
            }
            break *first;
        }
    };
    let on_store = sim
        .net
        .inner
        .lock()
        .records
        .values()
        .filter(|r| {
            r.replicas
                .iter()
                .any(|x| x.store == store && x.values[0].is_some())
        })
        .count();
    assert_eq!(on_store, REMOTE_MAX_RECORDS);
    assert_eq!(copies(&sim, &keys[0], 0), 5, "the used record stays");
    // The local copy stays; a rehydration stores the record again.
    let key = keys[evicted].clone();
    assert_eq!(copies(&sim, &key, 0), 4);
    assert!(a.get(&key, 0, true).await.unwrap().is_some());
    a.open(&key, None).await.unwrap();
    tick(&sim);
    tick(&sim);
    assert_eq!(copies(&sim, &key, 0), 5);
}
