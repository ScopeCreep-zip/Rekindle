//! A transaction get reaches a newer value a minority of the closest
//! replicas holds, which a forced get settles past ([`super::dht`]'s
//! `transaction` module, 0.5.8 `outbound_transaction_manager/mod.rs:1947-1966`
//! against `get_value.rs:49-88`).

use std::sync::Arc;

use rekindle_records::port::{DhtBackend, OpenIntent, RecordSchema};

use super::dht::SimDht;
use super::replicas::reachable;
use crate::Netsim;

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_transaction_get_reaches_a_minority_holders_newer_value() {
    let (sim, nodes) = Netsim::nodes(2, 41);
    let _pump = sim.start();
    let a = SimDht::new(0, Arc::clone(&sim.net), Arc::clone(&nodes[0].rng));
    let b = SimDht::new(1, Arc::clone(&sim.net), Arc::clone(&nodes[1].rng));
    let key = a
        .create(&RecordSchema::Dflt { owner_subkeys: 1 }, None)
        .await
        .unwrap()
        .key;
    a.set(&key, 0, vec![1], None).await.unwrap();
    {
        // The fourth closest replica alone holds seq 1, as a write that
        // landed only on replicas the other side of a partition leaves it.
        let mut guard = sim.net.inner.lock();
        let inner = &mut *guard;
        let record = inner.records.get_mut(&key).unwrap();
        let fourth = reachable(&record.replicas, &inner.faults, 1)[3];
        let mut newer = record.replicas[0].values[0].clone().unwrap();
        newer.seq += 1;
        newer.data = vec![2];
        record.replicas[fourth].values[0] = Some(newer);
    }
    b.open(&key, None, OpenIntent::Check).await.unwrap();
    let got = b.get(&key, 0, true).await.unwrap().unwrap();
    assert_eq!(got.data, vec![1], "a get stops at three holders of seq 0");
    let newest = b.get_newest(&key, 0).await.unwrap().unwrap();
    assert_eq!((newest.seq, newest.data), (got.seq + 1, vec![2]));
    let local = b.get(&key, 0, false).await.unwrap().unwrap();
    assert_eq!(local.data, vec![1], "a transaction stores nothing locally");
}

/// An open anonymous read holds one of the record's slots on each accepter
/// until its rollback, and never stops the owner's set
/// (`inbound_transactions_interface.rs:100-133`, `record_store/mod.rs:437-460`).
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn reads_take_anonymous_slots_and_never_hold_off_the_writer() {
    let (sim, nodes) = Netsim::nodes(2, 43);
    let _pump = sim.start();
    let a = SimDht::new(0, Arc::clone(&sim.net), Arc::clone(&nodes[0].rng));
    let b = SimDht::new(1, Arc::clone(&sim.net), Arc::clone(&nodes[1].rng));
    let key = a
        .create(&RecordSchema::Dflt { owner_subkeys: 1 }, None)
        .await
        .unwrap()
        .key;
    a.set(&key, 0, vec![1], None).await.unwrap();
    b.open(&key, None, OpenIntent::Check).await.unwrap();
    b.get_newest(&key, 0).await.unwrap();
    let held = |sim: &Netsim| {
        let inner = sim.net.inner.lock();
        inner.records[&key]
            .replicas
            .iter()
            .map(|r| r.open_reads)
            .sum::<u32>()
    };
    assert_eq!(held(&sim), 0, "the rollback releases every slot");
    {
        let mut inner = sim.net.inner.lock();
        for r in &mut inner.records.get_mut(&key).unwrap().replicas {
            r.open_reads = super::dht::transaction::PUBLIC_TRANSACTION_LIMIT;
        }
    }
    assert!(
        b.get_newest(&key, 0).await.is_err(),
        "every replica's anonymous slots are taken"
    );
    let written = a.set(&key, 0, vec![2], None).await;
    assert!(written.is_ok(), "open reads never refuse the owner's set");
}
