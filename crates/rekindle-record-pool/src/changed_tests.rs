//! A read that stored a newer value of a watched subkey publishes the change
//! it absorbed (lead decision 57, finding RK3; `changed.rs`).

use std::sync::Arc;

use rekindle_records::port::{OpenIntent, RecordPort, RecordSchema};

use crate::test_support::{pool_on, MemDht};
use crate::READ_CHANGE_COUNT;

const DFLT_2: RecordSchema = RecordSchema::Dflt { owner_subkeys: 2 };

/// A peer's record we watch, opened by its key.
async fn watched(
    dht: &Arc<MemDht>,
) -> (
    Arc<crate::RecordPool>,
    rekindle_records::lease::LeaseId,
    String,
) {
    let creator = pool_on(Arc::clone(dht));
    let (_, key, _) = RecordPort::create(&*creator, &DFLT_2, None).await.unwrap();
    let pool = pool_on(Arc::clone(dht));
    let lease = pool.acquire(&key, None, OpenIntent::Hold).await.unwrap();
    pool.watch(lease, [1].into_iter().collect()).await.unwrap();
    (pool, lease, key)
}

/// Seed 347: a history read in flight when the peer posted is answered
/// with the post and stores it locally, so Veilid reports nothing about
/// it afterwards (0.5.8 `watch_value.rs:2204-2229`, `:1455-1460`). The
/// read publishes the change, once, as a live change (not a watch death).
#[tokio::test]
async fn a_read_that_stored_a_peers_new_value_publishes_the_change() {
    let dht = Arc::new(MemDht::default());
    let (pool, lease, key) = watched(&dht).await;
    dht.write_remote(&key, 1, b"post", "bea");

    let read = pool.get(lease, 1, true).await.unwrap().expect("the post");
    assert_eq!(read.data, b"post");
    let published = dht.published();
    assert_eq!(published.len(), 1, "the absorbed change is published");
    let change = &published[0];
    assert_eq!(
        (change.key.as_str(), change.subkeys.as_slice()),
        (key.as_str(), &[1][..])
    );
    assert_eq!(change.value.as_ref().map(|v| v.seq), Some(0));
    assert_eq!(change.count, READ_CHANGE_COUNT);
    let report = pool.on_value_change(change);
    assert!(
        report.held && !report.watch_died,
        "a live change, no re-arm"
    );

    // The same value again, from the network or locally: nothing new.
    pool.get(lease, 1, true).await.unwrap();
    pool.get(lease, 1, false).await.unwrap();
    assert_eq!(dht.published().len(), 1, "published once");
}

/// An inspect-planned read (`read_changed`) is a reader too.
#[tokio::test]
async fn a_planned_read_of_a_newer_network_seq_publishes_the_change() {
    let dht = Arc::new(MemDht::default());
    let (pool, lease, key) = watched(&dht).await;
    dht.write_remote(&key, 1, b"post", "bea");
    let values = pool.read_changed(lease, &[1]).await.unwrap();
    assert_eq!(values.len(), 1);
    assert_eq!(dht.published().len(), 1);
}

/// A hinted read is the handler of the change that named the subkey, and
/// an unwatched subkey has no consumer: neither publishes.
#[tokio::test]
async fn a_hinted_read_or_an_unwatched_subkey_publishes_nothing() {
    let dht = Arc::new(MemDht::default());
    let (pool, lease, key) = watched(&dht).await;
    dht.write_remote(&key, 1, b"post", "bea");
    pool.read_hinted(lease, &[1]).await.unwrap();
    dht.write_remote(&key, 0, b"card", "bea");
    pool.get(lease, 0, true).await.unwrap();
    assert!(dht.published().is_empty());
}
