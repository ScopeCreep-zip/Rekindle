//! Own-write republish (plan E3.4 OW1, OW2, `republish.rs`): each round
//! inspects a record once and re-sends at its seq only the subkeys the
//! network holds behind ours, on the 60 s, doubling schedule; it stops once
//! a newer value replaced the write, never writing the older bytes over it.

use std::sync::Arc;
use std::time::Duration;

use rekindle_records::port::{RecordPort, RecordSchema, SetOutcome};

use crate::test_support::{pool_on, MemDht};

const DFLT_2: RecordSchema = RecordSchema::Dflt { owner_subkeys: 2 };

#[tokio::test(start_paused = true)]
async fn an_unheld_own_write_is_re_sent_at_its_seq_then_only_inspected() {
    let dht = Arc::new(MemDht::default());
    let pool = pool_on(Arc::clone(&dht));
    let (lease, key, _) = RecordPort::create(&*pool, &DFLT_2, None).await.unwrap();
    let outcome = pool
        .set_republished(lease, 1, b"left".to_vec(), None)
        .await
        .unwrap();
    assert_eq!(outcome, SetOutcome::Landed);
    pool.release(lease).await;
    // The write stored on too few nodes: the network holds none of it.
    dht.unhold(&key, 1);
    let seq = dht.value(&key, 1).unwrap().seq;
    let (sets, inspects) = (dht.sets(), dht.inspects());
    assert_eq!(pool.republishing(), [(key.clone(), 1)]);

    tokio::time::sleep(Duration::from_secs(59)).await;
    assert_eq!(dht.sets(), sets, "nothing before 60 s");
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(dht.inspects(), inspects + 1, "one inspect at 60 s");
    assert_eq!(dht.sets(), sets + 1, "the unheld subkey re-sent at 60 s");
    tokio::time::sleep(Duration::from_secs(118)).await;
    assert_eq!(dht.inspects(), inspects + 1, "the next round waits 120 s");
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert_eq!(dht.inspects(), inspects + 2, "inspected 120 s after");
    assert_eq!(dht.sets(), sets + 1, "landed now: no set");
    assert_eq!(pool.republishing(), [(key.clone(), 1)], "still checked");
    let stored = dht.value(&key, 1).unwrap();
    assert_eq!(
        (stored.seq, stored.data.as_slice()),
        (seq, b"left".as_slice())
    );
}

#[tokio::test(start_paused = true)]
async fn a_landed_record_costs_one_inspect_and_no_set_per_round() {
    let dht = Arc::new(MemDht::default());
    let pool = pool_on(Arc::clone(&dht));
    let schema = RecordSchema::Dflt { owner_subkeys: 8 };
    let (lease, _, _) = RecordPort::create(&*pool, &schema, None).await.unwrap();
    for subkey in 0..8 {
        pool.set_republished(lease, subkey, subkey.to_le_bytes().to_vec(), None)
            .await
            .unwrap();
    }
    pool.release(lease).await;
    let (sets, inspects) = (dht.sets(), dht.inspects());
    tokio::time::sleep(Duration::from_secs(61)).await;
    assert_eq!(dht.inspects(), inspects + 1, "one inspect for 8 subkeys");
    assert_eq!(dht.sets(), sets, "every subkey is held: nothing re-sent");
    assert_eq!(pool.republishing().len(), 8, "the schedule goes on");
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
    dht.unhold(&key, 1);
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
async fn a_failed_inspect_sends_nothing_and_keeps_the_write_due() {
    let dht = Arc::new(MemDht::default());
    let pool = pool_on(Arc::clone(&dht));
    let (lease, key, _) = RecordPort::create(&*pool, &DFLT_2, None).await.unwrap();
    pool.set_durable(lease, 0, b"profile".to_vec())
        .await
        .unwrap();
    pool.release(lease).await;
    assert_eq!(pool.republishing(), [(key.clone(), 0)]);
    dht.unhold(&key, 0);
    dht.fail_inspects(true);
    let sets = dht.sets();
    tokio::time::sleep(Duration::from_secs(61)).await;
    assert_eq!(dht.sets(), sets, "no blind re-send");
    assert_eq!(pool.republishing(), [(key.clone(), 0)], "still due");
    dht.fail_inspects(false);
    tokio::time::sleep(Duration::from_secs(121)).await;
    assert_eq!(dht.sets(), sets + 1, "the next round re-sends it");
}
