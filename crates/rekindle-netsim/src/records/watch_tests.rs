//! The watch model's rules (plan E2.B S5), each against the Veilid lines
//! [`super::watches`] cites.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use rekindle_records::port::{
    DhtBackend, DhtErrorKind, PortEvent, RecordSchema, RecordValue, ValueChange,
};

use super::dht::SimDht;
use super::watches::{self, Desired, MEMBER_WATCH_LIMIT, PUBLIC_WATCH_LIMIT, WATCH_TICK};
use crate::{Netsim, Node};

fn dht(sim: &Netsim, node: &Node) -> SimDht {
    SimDht::new(node.index, Arc::clone(&sim.net), Arc::clone(&node.rng))
}

/// A record node 0 created and seeded at subkeys 0..4, open on node 1, and
/// both nodes' backends.
async fn seeded(sim: &Netsim, nodes: &[Node]) -> (String, SimDht, SimDht) {
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
    (key, a, b)
}

/// Run the next watch tick and deliver; what node `i` heard.
fn tick(sim: &Netsim, nodes: &mut [Node], i: usize) -> Vec<ValueChange> {
    sim.net.inner.lock().next_tick = Some(tokio::time::Instant::now());
    sim.net.deliver();
    let mut heard = Vec::new();
    while let Ok(PortEvent::ValueChange(change)) = nodes[i].events.try_recv() {
        heard.push(change);
    }
    heard
}

fn dead(change: &ValueChange) -> bool {
    change.subkeys.is_empty() && change.count == 0 && change.value.is_none()
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn one_changed_subkey_carries_its_value_stored_before_the_report() {
    let (sim, mut nodes) = Netsim::nodes(2, 11);
    let (key, a, b) = seeded(&sim, &nodes).await;
    assert!(b.watch(&key, None, None, None).await.unwrap());
    a.set(&key, 2, b"news".to_vec(), None).await.unwrap();
    let heard = tick(&sim, &mut nodes, 1);
    let first = heard.first().expect("a change");
    assert_eq!(first.subkeys, vec![2]);
    let value = first.value.clone().expect("the value inline");
    assert_eq!((value.seq, value.data.as_slice()), (1, &b"news"[..]));
    let local = sim.net.inner.lock().nodes[1].local[&(key.clone(), 2)].clone();
    assert_eq!(local, value);
    // The other replicas' reports of it are not newer: nothing more.
    assert_eq!(heard.len(), 1, "{heard:?}");
    // A same-data re-push stores nothing and marks nothing.
    a.set(&key, 2, b"news".to_vec(), None).await.unwrap();
    assert!(tick(&sim, &mut nodes, 1).is_empty());
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn several_changed_subkeys_are_a_range_with_no_value() {
    let (sim, mut nodes) = Netsim::nodes(2, 12);
    let (key, a, b) = seeded(&sim, &nodes).await;
    b.watch(&key, None, None, None).await.unwrap();
    a.set(&key, 1, b"x".to_vec(), None).await.unwrap();
    a.set(&key, 3, b"y".to_vec(), None).await.unwrap();
    let heard = tick(&sim, &mut nodes, 1);
    assert!(!heard.is_empty());
    for change in &heard {
        assert_eq!(
            (change.subkeys.as_slice(), &change.value),
            (&[1, 3][..], &None)
        );
    }
    assert_eq!(
        sim.net.inner.lock().nodes[1]
            .local
            .get(&(key, 1))
            .map(|v| v.seq),
        None,
        "a range stores nothing"
    );
}

fn desired(subkeys: &[u32]) -> Desired {
    Desired {
        subkeys: subkeys.iter().copied().collect(),
        expiration: None,
        count: u32::MAX,
        member: None,
    }
}

fn value(seq: u32) -> RecordValue {
    RecordValue {
        seq,
        data: vec![1],
        writer: "w".into(),
    }
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn the_watcher_drops_a_value_it_may_not_report() {
    let (sim, _nodes) = Netsim::nodes(1, 13);
    let mut inner = sim.net.inner.lock();
    let k = "k".to_string();
    inner.nodes[0]
        .watches
        .insert(k.clone(), Some(desired(&[2, 3])));
    let set = |s: &[u32]| s.iter().copied().collect::<BTreeSet<u32>>();
    // The first changed subkey is not watched: the range, no value.
    let change = watches::receive(&mut inner, 0, &k, (set(&[1, 2]), 9, None)).unwrap();
    assert_eq!((change.subkeys, change.value), (vec![2], None));
    let change = watches::receive(&mut inner, 0, &k, (set(&[1]), 9, Some(value(0))));
    assert!(change.is_none(), "nothing watched changed");
    // Not newer than the local copy: dropped, and nothing is left.
    inner.nodes[0].local.insert((k.clone(), 2), value(4));
    assert!(watches::receive(&mut inner, 0, &k, (set(&[2]), 9, Some(value(4)))).is_none());
    // Newer: stored, then reported.
    let change = watches::receive(&mut inner, 0, &k, (set(&[2]), 9, Some(value(5)))).unwrap();
    assert_eq!(change.value, Some(value(5)));
    assert_eq!(inner.nodes[0].local[&(k.clone(), 2)], value(5));
    // A node that watches nothing hears nothing.
    inner.nodes[0].watches.clear();
    assert!(watches::receive(&mut inner, 0, &k, (set(&[2]), 9, Some(value(6)))).is_none());
}

/// The replicas holding `node`'s watch on `key`.
fn homes(inner: &crate::network::Inner, key: &str, node: usize) -> Vec<usize> {
    let replicas = &inner.records[key].replicas;
    (0..replicas.len())
        .filter(|r| replicas[*r].watches.iter().any(|w| w.node == node))
        .collect()
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_replica_refuses_watches_past_the_limits() {
    let (sim, nodes) = Netsim::nodes(PUBLIC_WATCH_LIMIT + 2, 14);
    let (key, _a, _b) = seeded(&sim, &nodes).await;
    let now = sim.net.now_us();
    {
        let mut inner = sim.net.inner.lock();
        for node in 1..=PUBLIC_WATCH_LIMIT + 1 {
            watches::desire(&mut inner, node, &key, Some(desired(&[0])), now);
            assert_eq!(watches::held(&inner, node, &key), 3);
        }
        assert_eq!(homes(&inner, &key, 1), vec![0, 1, 2]);
        assert_eq!(homes(&inner, &key, PUBLIC_WATCH_LIMIT + 1), vec![3, 4, 5]);
    }
    // A member has its own budget per member (a fresh record).
    let (key, _a, _b) = seeded(&sim, &nodes).await;
    let mut inner = sim.net.inner.lock();
    let owner = inner.records[&key].owner;
    for node in 0..=MEMBER_WATCH_LIMIT {
        let member = Desired {
            member: Some(owner),
            ..desired(&[0])
        };
        watches::desire(&mut inner, node, &key, Some(member), now);
    }
    assert_eq!(homes(&inner, &key, MEMBER_WATCH_LIMIT), vec![3, 4, 5]);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn expiration_is_checked_clamped_and_ends_the_watch() {
    let (sim, mut nodes) = Netsim::nodes(2, 15);
    let (key, _a, b) = seeded(&sim, &nodes).await;
    let now = sim.net.now_us();
    let err = b.watch(&key, None, Some(now + 1_000_000), None).await;
    assert_eq!(err.unwrap_err().kind, DhtErrorKind::InvalidArgument);
    let at = sim.net.now_us() + 20_000_000;
    assert!(b.watch(&key, None, Some(at), None).await.unwrap());
    assert!(tick(&sim, &mut nodes, 1).is_empty());
    tokio::time::advance(Duration::from_secs(20)).await;
    let heard = tick(&sim, &mut nodes, 1);
    assert!(heard.len() == 1 && dead(&heard[0]), "{heard:?}");
    assert_eq!(watches::held(&sim.net.inner.lock(), 1, &key), 0);
    // With no expiration the replicas clamp to the max and the node renews;
    // a node offline past the max loses them, and gets them back online.
    b.watch(&key, None, None, None).await.unwrap();
    sim.net.inner.lock().faults.dht_offline.insert(1);
    tokio::time::advance(watches::MAX_WATCH_EXPIRATION + WATCH_TICK).await;
    tick(&sim, &mut nodes, 1);
    assert_eq!(watches::held(&sim.net.inner.lock(), 1, &key), 0);
    sim.net.inner.lock().faults.dht_offline.remove(&1);
    tick(&sim, &mut nodes, 1);
    assert_eq!(watches::held(&sim.net.inner.lock(), 1, &key), 3);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn death_is_empty_subkeys_count_zero_no_value() {
    let (sim, mut nodes) = Netsim::nodes(2, 16);
    let (key, a, b) = seeded(&sim, &nodes).await;
    b.watch(&key, Some(&[1]), None, None).await.unwrap();
    sim.fault(crate::Fault::WatchDeath {
        node: 1,
        key: key.clone(),
    });
    sim.net.deliver();
    assert!(matches!(nodes[1].events.try_recv(), Ok(PortEvent::ValueChange(c)) if dead(&c)));
    // A cancel ends it at the next tick, the same way.
    b.watch(&key, Some(&[1]), None, None).await.unwrap();
    assert!(!b.cancel_watch(&key, Some(&[1])).await.unwrap());
    let heard = tick(&sim, &mut nodes, 1);
    assert!(heard.len() == 1 && dead(&heard[0]), "{heard:?}");
    // A detached node hears nothing of a change.
    b.watch(&key, None, None, None).await.unwrap();
    sim.net.inner.lock().faults.detached.insert(1);
    a.set(&key, 1, b"z".to_vec(), None).await.unwrap();
    assert!(tick(&sim, &mut nodes, 1).is_empty());
}
