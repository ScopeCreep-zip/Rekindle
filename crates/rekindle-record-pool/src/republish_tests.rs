//! Own-write republish (plan E3.4 OW1, `republish.rs`): a write that stored
//! is re-sent at its seq at 60 s, then 120 s later, and stops once a newer
//! value replaced it, never writing the older bytes over the newer.

use std::sync::Arc;
use std::time::Duration;

use rekindle_records::port::{RecordPort, RecordSchema, SetOutcome};

use crate::test_support::{pool_on, MemDht};

const DFLT_2: RecordSchema = RecordSchema::Dflt { owner_subkeys: 2 };

#[tokio::test(start_paused = true)]
async fn a_stored_own_write_is_re_sent_at_its_seq_on_the_schedule() {
    let dht = Arc::new(MemDht::default());
    let pool = pool_on(Arc::clone(&dht));
    let (lease, key, _) = RecordPort::create(&*pool, &DFLT_2, None).await.unwrap();
    let outcome = pool
        .set_republished(lease, 1, b"left".to_vec(), None)
        .await
        .unwrap();
    assert_eq!(outcome, SetOutcome::Landed);
    pool.release(lease).await;
    let seq = dht.value(&key, 1).unwrap().seq;
    let sets = dht.sets();
    assert_eq!(pool.republishing(), [(key.clone(), 1)]);

    tokio::time::sleep(Duration::from_secs(59)).await;
    assert_eq!(dht.sets(), sets, "nothing before 60 s");
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(dht.sets(), sets + 1, "re-sent at 60 s");
    tokio::time::sleep(Duration::from_secs(118)).await;
    assert_eq!(dht.sets(), sets + 1, "the next waits 120 s");
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert_eq!(dht.sets(), sets + 2, "re-sent 120 s after the first");
    let stored = dht.value(&key, 1).unwrap();
    assert_eq!(
        (stored.seq, stored.data.as_slice()),
        (seq, b"left".as_slice())
    );
}

#[tokio::test(start_paused = true)]
async fn a_newer_write_stops_the_re_sends_of_the_older() {
    let dht = Arc::new(MemDht::default());
    let pool = pool_on(Arc::clone(&dht));
    let (lease, key, _) = RecordPort::create(&*pool, &DFLT_2, None).await.unwrap();
    pool.set_republished(lease, 1, b"old".to_vec(), None)
        .await
        .unwrap();
    // A present-tense write of newer bytes: not re-sent itself, and the
    // older value must never be written over it.
    pool.set(lease, 1, b"new".to_vec(), None).await.unwrap();
    let newer = dht.value(&key, 1).map(|v| (v.seq, v.data)).unwrap();
    let sets = dht.sets();
    tokio::time::sleep(Duration::from_secs(61)).await;
    assert_eq!(dht.sets(), sets, "the older bytes were not re-sent");
    assert!(
        pool.republishing().is_empty(),
        "the superseded entry is gone"
    );
    assert_eq!(dht.value(&key, 1).map(|v| (v.seq, v.data)), Some(newer));
}

#[tokio::test(start_paused = true)]
async fn a_durable_write_that_landed_is_re_sent_too() {
    let dht = Arc::new(MemDht::default());
    let pool = pool_on(Arc::clone(&dht));
    let (lease, key, _) = RecordPort::create(&*pool, &DFLT_2, None).await.unwrap();
    pool.set_durable(lease, 0, b"profile".to_vec())
        .await
        .unwrap();
    assert_eq!(pool.republishing(), [(key, 0)]);
    let sets = dht.sets();
    tokio::time::sleep(Duration::from_secs(61)).await;
    assert_eq!(dht.sets(), sets + 1);
}
