//! A read's cost is decided when its fanout starts, after its subkey lock
//! and gate (0.5.8 `storage_manager/get_value.rs:302-303` takes the lock,
//! `:178-189` the gate, then `outbound_get_value` asks the replicas), not
//! when it was queued.

use std::sync::Arc;

use rekindle_records::port::{DhtBackend, OpenIntent, RecordSchema};
use tokio::time::Instant;

use super::dht::SimDht;
use crate::latency::{DEADLINE, GET};
use crate::{Netsim, Node};

fn dht(sim: &Netsim, node: &Node) -> SimDht {
    SimDht::new(node.index, Arc::clone(&sim.net), Arc::clone(&node.rng))
}

/// Node 1 reads subkeys 2 and 4 of node 0's record while nothing is
/// written there: the first read of 4 runs to the deadline. A second read
/// of 4 (a coalesced `[2, 4]` hint's) queues behind it; node 0 writes 4
/// meanwhile, so the queued read's fanout, starting when the first ends,
/// finds the value at a get's cost rather than a second deadline.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_read_queued_behind_a_deadline_read_finds_a_value_written_meanwhile() {
    let (sim, nodes) = Netsim::nodes(2, 77);
    let (a, b) = (dht(&sim, &nodes[0]), dht(&sim, &nodes[1]));
    let key = a
        .create(&RecordSchema::Dflt { owner_subkeys: 6 }, None)
        .await
        .unwrap()
        .key;
    a.set(&key, 0, vec![1], None).await.unwrap();
    b.open(&key, None, OpenIntent::Hold).await.unwrap();
    let start = Instant::now();
    let first = async {
        let read = b.get(&key, 4, true).await.unwrap();
        (read, Instant::now() - start)
    };
    let queued = async {
        tokio::task::yield_now().await;
        let read = b.get(&key, 4, true).await.unwrap();
        (read, Instant::now() - start)
    };
    let write = async {
        tokio::task::yield_now().await;
        a.set(&key, 4, b"CS2".to_vec(), None).await.unwrap();
        Instant::now() - start
    };
    let ((_, first_at), (queued, queued_at), written_at) = tokio::join!(first, queued, write);
    assert!(
        first_at >= DEADLINE.min,
        "the first read found no value at its start: {first_at:?}"
    );
    assert!(written_at < first_at, "written at {written_at:?}");
    assert_eq!(queued.map(|v| v.data), Some(b"CS2".to_vec()));
    assert!(
        queued_at <= first_at + GET.max,
        "the queued read began its fanout after the write and cost a get, not a \
         deadline: done {queued_at:?}, the first at {first_at:?}"
    );
}
