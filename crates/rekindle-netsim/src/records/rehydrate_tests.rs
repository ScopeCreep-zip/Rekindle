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

/// A record `a` created and wrote in an earlier session: a record created
/// this session is never rehydrated by a reopen (0.5.8
/// `open_record.rs:62-71`), so the session's mark is cleared as a restart
/// would.
async fn seeded(sim: &Netsim, a: &SimDht) -> String {
    let key = a
        .create(&RecordSchema::Dflt { owner_subkeys: 2 }, None)
        .await
        .unwrap()
        .key;
    a.set(&key, 0, vec![7], None).await.unwrap();
    sim.net.inner.lock().nodes[0].created.clear();
    key
}

/// VU2 item 6: a reopen queues a rehydration only when asked, for a record
/// not created this session, past the 300 s recheck window of its last
/// check (0.5.8 `open_record.rs:62-81`, `rehydrate.rs:26-37`,
/// `storage_manager/mod.rs:198`).
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_reopen_rehydrates_only_when_asked_unmade_here_and_past_the_recheck() {
    let (sim, nodes) = Netsim::nodes(2, 34);
    let a = dht(&sim, &nodes[0]);
    let queued = |sim: &Netsim, key: &str| sim.net.inner.lock().nodes[0].rehydrations.contains(key);
    let made_here = a
        .create(&RecordSchema::Dflt { owner_subkeys: 1 }, None)
        .await
        .unwrap()
        .key;
    a.open(&made_here, None, rekindle_records::port::OpenIntent::Hold)
        .await
        .unwrap();
    assert!(!queued(&sim, &made_here), "created this session");

    let key = seeded(&sim, &a).await;
    a.open(&key, None, rekindle_records::port::OpenIntent::Check)
        .await
        .unwrap();
    assert!(!queued(&sim, &key), "not asked");
    a.open(&key, None, rekindle_records::port::OpenIntent::Hold)
        .await
        .unwrap();
    assert!(queued(&sim, &key));
    tick(&sim);
    assert!(!queued(&sim, &key), "checked");
    tokio::time::advance(
        super::rehydrate::REHYDRATE_RECHECK.saturating_sub(std::time::Duration::from_secs(1)),
    )
    .await;
    a.open(&key, None, rekindle_records::port::OpenIntent::Hold)
        .await
        .unwrap();
    assert!(!queued(&sim, &key), "inside the recheck window");
    tokio::time::advance(std::time::Duration::from_secs(1)).await;
    a.open(&key, None, rekindle_records::port::OpenIntent::Hold)
        .await
        .unwrap();
    assert!(queued(&sim, &key), "past it");
}

/// VU2 item 5: with no replica reached, an inspect and an open of an
/// unknown record answer `TryAgain`, not an empty report or `KeyNotFound`
/// (0.5.8 `inspect_record.rs:346-349`, `open_record.rs:101-104`); a
/// rehydration keeps its request.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn no_replica_reached_is_try_again() {
    let (sim, nodes) = Netsim::nodes(2, 35);
    let (a, b) = (dht(&sim, &nodes[0]), dht(&sim, &nodes[1]));
    let key = seeded(&sim, &a).await;
    let unknown = b
        .create(&RecordSchema::Dflt { owner_subkeys: 1 }, None)
        .await
        .unwrap()
        .key;
    {
        let mut inner = sim.net.inner.lock();
        for replica in &mut inner.records.get_mut(&key).unwrap().replicas {
            replica.home = 1;
        }
        for replica in &mut inner.records.get_mut(&unknown).unwrap().replicas {
            replica.home = 1;
        }
    }
    sim.fault(Fault::Partition {
        a: vec![0],
        b: vec![1],
    });
    let err = a
        .inspect(&key, None, rekindle_records::port::ReportScope::SyncGet)
        .await
        .unwrap_err();
    assert_eq!(err.kind, DhtErrorKind::TryAgain);
    let err = a
        .open(&unknown, None, rekindle_records::port::OpenIntent::Hold)
        .await
        .unwrap_err();
    assert_eq!(err.kind, DhtErrorKind::TryAgain);
    a.open(&key, None, rekindle_records::port::OpenIntent::Hold)
        .await
        .unwrap();
    tick(&sim);
    assert!(sim.net.inner.lock().nodes[0].rehydrations.contains(&key));
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn an_open_of_a_held_record_pushes_what_the_replicas_lost() {
    let (sim, nodes) = Netsim::nodes(2, 31);
    let a = dht(&sim, &nodes[0]);
    let key = seeded(&sim, &a).await;
    wipe(&sim, &key);
    a.open(&key, None, rekindle_records::port::OpenIntent::Hold)
        .await
        .unwrap();
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
    let key = seeded(&sim, &a).await;
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
    a.open(&key, None, rekindle_records::port::OpenIntent::Hold)
        .await
        .unwrap();
    tick(&sim);
    tick(&sim);
    assert_eq!(copies(&sim, &key, 1), 0);
    assert!(a.get(&key, 1, false).await.unwrap().is_none());
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn an_offline_open_rehydrates_once_back_online() {
    let (sim, nodes) = Netsim::nodes(2, 33);
    let a = dht(&sim, &nodes[0]);
    let key = seeded(&sim, &a).await;
    wipe(&sim, &key);
    sim.fault(Fault::DhtOffline {
        node: 0,
        offline: true,
    });
    a.open(&key, None, rekindle_records::port::OpenIntent::Hold)
        .await
        .unwrap();
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
    let key = seeded(&sim, &a).await;
    wipe(&sim, &key);
    sim.fault(Fault::Partition {
        a: vec![0],
        b: (1..10).collect(),
    });
    a.open(&key, None, rekindle_records::port::OpenIntent::Hold)
        .await
        .unwrap();
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
    let mut keys = vec![seeded(&sim, &a).await];
    // Create records until a store is full; a read uses the first record
    // after every create, so each store's LRU is its oldest other record.
    let (evicted, store, had) = loop {
        assert!(keys.len() < 10 * REMOTE_MAX_RECORDS, "no store filled");
        let before: Vec<BTreeSet<usize>> = keys.iter().map(|k| stores_of(&sim, k)).collect();
        keys.push(seeded(&sim, &a).await);
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
            break (first.0, first.1, before[first.0].len());
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
    assert!(copies(&sim, &keys[0], 0) >= 5, "the used record stays");
    // The local copy stays; a rehydration stores the record again.
    let key = keys[evicted].clone();
    assert!(copies(&sim, &key, 0) < had, "evicted from a store");
    assert!(a.get(&key, 0, true).await.unwrap().is_some());
    a.open(&key, None, rekindle_records::port::OpenIntent::Hold)
        .await
        .unwrap();
    tick(&sim);
    tick(&sim);
    assert!(copies(&sim, &key, 0) >= 5);
}
