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
/// is still running; it returns at the end of its drawn span.
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
    tokio::time::sleep(latency::SET_RPC.max + Duration::from_millis(100)).await;
    assert!(!task.is_finished(), "the call is still running");
    assert!(
        holding(&sim, &record.key, b"v0", 0) >= 5,
        "the first wave landed"
    );
    assert_eq!(
        local_seq(&sim, 0, &record.key),
        None,
        "not local before return"
    );
    assert_eq!(task.await.unwrap().unwrap(), None);
    let took = start.elapsed();
    assert!(took >= latency::SET.min && took <= latency::SET.max + Duration::from_millis(30));
    assert_eq!(local_seq(&sim, 0, &record.key), Some(0));
}

/// One race of two writers at seq 1: how many replicas hold each value,
/// and what each call answered.
async fn race(seed: u64) -> (usize, usize, bool, bool) {
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
    let (ra, rb) = (ra.unwrap(), rb.unwrap());
    let unmoved =
        local_seq(&sim, 0, &record.key) == Some(0) && local_seq(&sim, 1, &record.key) == Some(0);
    (
        holding(&sim, &record.key, b"a", 1),
        holding(&sim, &record.key, b"b", 1),
        ra.is_none() && rb.is_none() && unmoved,
        ra.is_some() || rb.is_some(),
    )
}

/// MED-2: two writers' same-seq fanouts interleave replica by replica, in
/// arrival order drawn from the seed; a split leaves neither value at
/// consensus, both calls below it with their local seqs unmoved.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn concurrent_same_seq_sets_can_split_with_neither_at_consensus() {
    let mut split = None;
    for seed in 1..=40 {
        let (na, nb, below, landed) = race(seed).await;
        if below && na > 0 && nb > 0 {
            assert!(!landed, "seed {seed}: neither call landed");
            split = Some((seed, na, nb));
            break;
        }
    }
    let (seed, na, nb) = split.expect("some seed splits the replicas");
    let again = race(seed).await;
    assert_eq!((again.0, again.1), (na, nb), "one seed, one split");
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
