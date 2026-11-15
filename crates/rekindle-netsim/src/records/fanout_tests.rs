//! A set's timed fanout (plan E2.B S11, MED-1 and MED-2; `fanout.rs` cites
//! veilid-core).

use std::sync::Arc;
use std::time::Duration;

use rekindle_records::port::{DhtBackend, RecordSchema, WriterKey};

use super::dht::SimDht;
use crate::latency;
use crate::Netsim;

fn dht(sim: &Netsim, nodes: &[crate::Node], node: usize) -> SimDht {
    SimDht::new(node, Arc::clone(&sim.net), Arc::clone(&nodes[node].rng))
}

fn holding(sim: &Netsim, key: &str, data: &[u8], seq: u32) -> usize {
    sim.replica_values(key, 0)
        .into_iter()
        .flatten()
        .filter(|v| v.seq == seq && v.data == data)
        .count()
}

fn local_seq(sim: &Netsim, node: usize, key: &str) -> Option<u32> {
    sim.net.inner.lock().nodes[node]
        .local
        .get(&(key.to_string(), 0))
        .map(|v| v.seq)
}

/// MED-1: replicas hold a set's value as its RPCs arrive, while the call
/// is still running; it returns at its fifth accept (0.5.8: `set_value_count`
/// accepts end the fanout).
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_set_lands_on_replicas_before_the_call_returns() {
    let (sim, nodes) = Netsim::nodes(2, 71);
    let a = Arc::new(dht(&sim, &nodes, 0));
    let record = a
        .create(&RecordSchema::Dflt { owner_subkeys: 1 }, None)
        .await
        .unwrap();
    let start = tokio::time::Instant::now();
    let task = tokio::spawn({
        let (a, key) = (Arc::clone(&a), record.key.clone());
        async move { a.set(&key, 0, b"v0".to_vec(), None).await }
    });
    let mut seen_early = false;
    while !task.is_finished() {
        if holding(&sim, &record.key, b"v0", 0) > 0 && local_seq(&sim, 0, &record.key).is_none() {
            seen_early = true;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(seen_early, "replicas held it while the call ran");
    assert_eq!(task.await.unwrap().unwrap(), None);
    let took = start.elapsed();
    assert!(
        took >= latency::SET_RPC.min && took <= latency::SET_RPC.max + Duration::from_millis(20)
    );
    assert_eq!(local_seq(&sim, 0, &record.key), Some(0));
    // The throttle sent five, all accepted: no lane was left in flight
    // (0.5.8 `ThrottleAtConsensus`, `fanout_queue.rs:500-552`).
    tokio::time::sleep(latency::SET_RPC.max).await;
    assert_eq!(holding(&sim, &record.key, b"v0", 0), 5);
}

/// One race of two writers at seq 1: what each call answered and the value
/// each node holds after.
async fn race(seed: u64) -> (Option<Vec<u8>>, Option<Vec<u8>>, Vec<u8>, Vec<u8>) {
    let (sim, nodes) = Netsim::nodes(2, seed);
    let (a, b) = (dht(&sim, &nodes, 0), dht(&sim, &nodes, 1));
    let record = a
        .create(&RecordSchema::Dflt { owner_subkeys: 1 }, None)
        .await
        .unwrap();
    a.set(&record.key, 0, b"v0".to_vec(), None).await.unwrap();
    let owner = WriterKey {
        keypair: sim.owner_keypair(&record.key).unwrap(),
        public: String::new(),
    };
    b.open(&record.key, Some(&owner)).await.unwrap();
    b.get(&record.key, 0, true).await.unwrap();
    let (ra, rb) = tokio::join!(
        a.set(&record.key, 0, b"a".to_vec(), None),
        b.set(&record.key, 0, b"b".to_vec(), None),
    );
    let local = |node: usize| {
        sim.net.inner.lock().nodes[node]
            .local
            .get(&(record.key.clone(), 0))
            .map(|v| v.data.clone())
            .unwrap_or_default()
    };
    (
        ra.unwrap().map(|v| v.data),
        rb.unwrap().map(|v| v.data),
        local(0),
        local(1),
    )
}

/// VU2 item 1: two writers' same-seq fanouts no longer split the replicas
/// until the deadline (0.5.7 restarted on every rival); each tallies the
/// holders and switches to a rival held by more or closer replicas
/// (0.5.8 `set_value.rs:96-111`, `value_tally.rs:90-106`). A call that met
/// a rival returns `Some`, its own value included (`set_value.rs:753-760`).
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn concurrent_same_seq_sets_settle_by_the_tally() {
    let mut switched = None;
    for seed in 1..=40 {
        let (ra, rb, la, lb) = race(seed).await;
        // Each landed call met the other's value, so it reports what it
        // settled on, its local copy.
        for (r, l) in [(&ra, &la), (&rb, &lb)] {
            if let Some(r) = r {
                assert_eq!(r, l, "seed {seed}: the call returns what it stored");
            }
        }
        if ra.is_some() && rb.is_some() && la == lb {
            switched = Some((seed, la));
            break;
        }
    }
    let (seed, settled) = switched.expect("some seed has one writer switch to the other's value");
    let again = race(seed).await;
    assert_eq!(
        (again.2.clone(), again.3),
        (settled.clone(), settled),
        "one seed, one outcome"
    );
}

/// LV17 follow-up (ii): `Fault::DropFrom` is a one-direction app-message
/// loss; the DHT is a separate path. A node under it still sets values its
/// peers read, and reads what they set from replicas, at full rate.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn drop_from_leaves_the_dht_untouched() {
    let (sim, nodes) = Netsim::nodes(2, 73);
    let (a, b) = (dht(&sim, &nodes, 0), dht(&sim, &nodes, 1));
    sim.fault(crate::Fault::DropFrom {
        node: 1,
        per_mille: 1000,
    });
    let schema = RecordSchema::Dflt { owner_subkeys: 1 };
    let theirs = b.create(&schema, None).await.unwrap().key;
    assert_eq!(b.set(&theirs, 0, b"b0".to_vec(), None).await.unwrap(), None);
    assert!(
        holding(&sim, &theirs, b"b0", 0) >= 5,
        "its set reached the replicas"
    );
    a.open(&theirs, None).await.unwrap();
    let read = a.get(&theirs, 0, true).await.unwrap().expect("its value");
    assert_eq!(read.data, b"b0");
    let ours = a.create(&schema, None).await.unwrap().key;
    a.set(&ours, 0, b"a0".to_vec(), None).await.unwrap();
    b.open(&ours, None).await.unwrap();
    let read = b.get(&ours, 0, true).await.unwrap().expect("our value");
    assert_eq!(read.data, b"a0");
}

fn cipher(seq: u32, data: &[u8], nonce: u64) -> super::replicas::Cipher {
    super::replicas::Cipher {
        value: rekindle_records::port::RecordValue {
            seq,
            data: data.to_vec(),
            writer: "w".into(),
        },
        nonce,
    }
}

/// `replicas` 0 and 1 hold `held`; the rest are empty.
fn two_holding(held: &super::replicas::Cipher) -> Vec<super::replicas::Replica> {
    let mut replicas = super::replicas::place("k", 10, 1);
    for r in &mut replicas[..2] {
        r.put(0, held);
    }
    replicas
}

/// VU2 item 1: ours wins a same-seq split by holders, and the fanout says
/// a rival is held, so the call returns `Some(ours)` (0.5.8
/// `set_value.rs:753-760`, `value_tally.rs:52-57`).
#[test]
fn ours_settles_over_a_minority_rival_and_reports_the_conflict() {
    let schema = RecordSchema::Dflt { owner_subkeys: 1 };
    let mut replicas = two_holding(&cipher(0, b"rival", 1));
    let ours = cipher(0, b"ours", 2);
    // Three far replicas take ours before a rival holder answers.
    let far = [9, 8, 7, 0, 6, 5, 4, 3, 2, 1];
    let fanout = super::fanout::set_now(&schema, &mut replicas, &far, 0, &ours);
    assert_eq!(fanout.value, ours);
    assert!(fanout.consensus && fanout.conflicted);
}

/// VU2 item 9: a rewrite of the same value with no local copy is a fresh
/// encryption, a rival of the bytes the replicas hold (0.5.8
/// `record_encryption.rs:43`); the same bytes are accepted as is.
#[test]
fn a_fresh_encryption_of_the_held_value_is_a_rival() {
    let schema = RecordSchema::Dflt { owner_subkeys: 1 };
    let held = cipher(0, b"v", 1);
    let far = [9, 8, 7, 0, 6, 5, 4, 3, 2, 1];
    let mut replicas = two_holding(&held);
    let fanout = super::fanout::set_now(&schema, &mut replicas, &far, 0, &cipher(0, b"v", 2));
    assert!(fanout.conflicted, "two encryptions at one seq");
    let mut replicas = two_holding(&held);
    let fanout = super::fanout::set_now(&schema, &mut replicas, &far, 0, &held);
    assert!(!fanout.conflicted, "the held bytes resent");
}

/// VU2 item 13: a set that ends exhausted at three or four accepts landed:
/// it is stored locally (0.5.8 `storage_manager/mod.rs:990-1030`).
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_set_exhausted_at_three_holders_lands() {
    let (sim, nodes) = Netsim::nodes(3, 41);
    let a = dht(&sim, &nodes, 0);
    let key = a
        .create(&RecordSchema::Dflt { owner_subkeys: 1 }, None)
        .await
        .unwrap()
        .key;
    sim.fault(crate::Fault::Partition {
        a: vec![0],
        b: vec![1, 2],
    });
    let reached = sim.net.inner.lock().records[&key]
        .replicas
        .iter()
        .filter(|r| r.home == 0)
        .count();
    assert!((3..5).contains(&reached), "{reached}");
    assert_eq!(a.set(&key, 0, b"v".to_vec(), None).await.unwrap(), None);
    assert_eq!(local_seq(&sim, 0, &key), Some(0));
}

/// VU2 item 2: a forced get takes a same-seq rival over the local copy
/// (one holder beats a local floor nobody holds), but not while a write of
/// ours waits; without force the local copy answers (0.5.8
/// `get_value.rs:44-90,600-659`).
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_same_seq_rival_replaces_the_local_copy_only_on_force_with_nothing_pending() {
    let (sim, nodes) = Netsim::nodes(2, 72);
    let a = dht(&sim, &nodes, 0);
    let key = a
        .create(&RecordSchema::Dflt { owner_subkeys: 1 }, None)
        .await
        .unwrap()
        .key;
    a.set(&key, 0, b"first".to_vec(), None).await.unwrap();
    let writer = sim.net.inner.lock().nodes[0].local[&(key.clone(), 0)]
        .writer
        .clone();
    {
        let mut inner = sim.net.inner.lock();
        for r in &mut inner.records.get_mut(&key).unwrap().replicas {
            r.values[0] = Some(rekindle_records::port::RecordValue {
                seq: 0,
                data: b"rival".to_vec(),
                writer: writer.clone(),
            });
        }
        inner.nodes[0].pushes.insert((key.clone(), 0), None);
    }
    assert_eq!(a.get(&key, 0, true).await.unwrap().unwrap().data, b"first");
    assert_eq!(a.get(&key, 0, false).await.unwrap().unwrap().data, b"first");
    sim.net.inner.lock().nodes[0].pushes.clear();
    assert_eq!(a.get(&key, 0, true).await.unwrap().unwrap().data, b"rival");
    assert_eq!(a.get(&key, 0, false).await.unwrap().unwrap().data, b"rival");
}
