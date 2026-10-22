//! Churn, equivocation, the inbound RPC queue and inspect truncation
//! (plan E2.B S11; `records/outside.rs` and `network.rs` cite veilid-core).

use std::sync::Arc;

use rekindle_records::port::{DhtBackend, PortEvent, RecordSchema, ReportScope};

use super::dht::SimDht;
use crate::{Fault, Netsim};

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn churned_replicas_lose_their_watches_and_the_watch_is_placed_again() {
    let (sim, nodes) = Netsim::nodes(3, 81);
    let a = SimDht::new(0, Arc::clone(&sim.net), Arc::clone(&nodes[0].rng));
    let b = SimDht::new(1, Arc::clone(&sim.net), Arc::clone(&nodes[1].rng));
    let record = a
        .create(&RecordSchema::Dflt { owner_subkeys: 1 }, None)
        .await
        .unwrap();
    a.set(&record.key, 0, b"v0".to_vec(), None).await.unwrap();
    b.open(&record.key, None).await.unwrap();
    assert!(b.watch(&record.key, None, None, None).await.unwrap());
    assert_eq!(
        crate::records::watches::held(&sim.net.inner.lock(), 1, &record.key),
        3
    );
    sim.fault(Fault::ReplicaChurn {
        key: record.key.clone(),
        count: 10,
    });
    assert!(sim
        .replica_values(&record.key, 0)
        .iter()
        .all(Option::is_none));
    assert_eq!(
        crate::records::watches::held(&sim.net.inner.lock(), 1, &record.key),
        0
    );
    sim.net.inner.lock().next_tick = Some(tokio::time::Instant::now());
    sim.net.deliver();
    assert_eq!(
        crate::records::watches::held(&sim.net.inner.lock(), 1, &record.key),
        3
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn an_equivocating_writer_splits_the_replicas_and_the_first_value_wins() {
    let (sim, nodes) = Netsim::nodes(2, 82);
    let a = SimDht::new(0, Arc::clone(&sim.net), Arc::clone(&nodes[0].rng));
    let record = a
        .create(&RecordSchema::Dflt { owner_subkeys: 1 }, None)
        .await
        .unwrap();
    let owner = sim.owner_keypair(&record.key).unwrap();
    let forge = |data: &[u8], to: &[usize]| {
        sim.forge_write_to(&record.key, 0, data.to_vec(), (&owner, 1), to)
            .unwrap()
    };
    assert_eq!(forge(b"x", &[0, 1, 2]), 3);
    assert_eq!(forge(b"y", &[3, 4, 5]), 3);
    assert_eq!(forge(b"z", &[0, 9]), 1, "replica 0 keeps its first value");
    let seen: Vec<Option<Vec<u8>>> = sim
        .replica_values(&record.key, 0)
        .into_iter()
        .map(|v| v.map(|v| v.data))
        .collect();
    assert_eq!(seen[0].as_deref(), Some(&b"x"[..]));
    assert_eq!(seen[3].as_deref(), Some(&b"y"[..]));
    assert_eq!(seen[9].as_deref(), Some(&b"z"[..]));
    assert!(sim
        .forge_write_to(&record.key, 0, vec![0; 32_769], (&owner, 2), &[0])
        .is_ok_and(|n| n == 0));
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn the_inbound_rpc_queue_holds_1024() {
    let (sim, mut nodes) = Netsim::nodes(2, 83);
    for i in 0..1100u32 {
        sim.stray_message(1, 0, i.to_le_bytes().to_vec());
    }
    sim.net.deliver();
    let mut heard = 0;
    while let Ok(PortEvent::AppMessage { .. }) = nodes[0].events.try_recv() {
        heard += 1;
    }
    assert_eq!((heard, sim.rpc_dropped(0)), (1024, 76));
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn an_inspect_is_truncated_to_the_schema() {
    let (sim, nodes) = Netsim::nodes(1, 84);
    let a = SimDht::new(0, Arc::clone(&sim.net), Arc::clone(&nodes[0].rng));
    let record = a
        .create(&RecordSchema::Dflt { owner_subkeys: 2 }, None)
        .await
        .unwrap();
    let report = a
        .inspect(&record.key, Some(&[1, 7, 0, 1]), ReportScope::Local)
        .await
        .unwrap();
    assert_eq!(report.subkeys, vec![0, 1]);
}
