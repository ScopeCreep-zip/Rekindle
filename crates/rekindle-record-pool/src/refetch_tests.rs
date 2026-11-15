//! D8's re-fetch cap through a real pool (`refetch.rs`, threat model D8): an
//! inflated inspect seq that no value confirms drives one forced get, not
//! one per inspect, while a value change still force-reads the subkey.

use std::sync::Arc;
use std::time::Duration;

use rekindle_records::lease::LeaseId;
use rekindle_records::port::{RecordPort, RecordSchema};

use crate::refetch::REFETCH_BACKOFF_MIN;
use crate::test_support::{pool_on, MemDht};
use crate::RecordPool;

const DFLT_2: RecordSchema = RecordSchema::Dflt { owner_subkeys: 2 };

/// A pool holding one record whose subkey 0 is written once (seq 0).
async fn written() -> (Arc<MemDht>, Arc<RecordPool>, LeaseId, String) {
    let dht = Arc::new(MemDht::default());
    let pool = pool_on(Arc::clone(&dht));
    let (lease, key, _) = RecordPort::create(&*pool, &DFLT_2, None).await.unwrap();
    pool.set(lease, 0, b"v0".to_vec(), None).await.unwrap();
    (dht, pool, lease, key)
}

/// Forced (network) gets of `key`'s subkey 0 so far.
fn fetches(dht: &MemDht, key: &str) -> usize {
    dht.gets()
        .iter()
        .filter(|(k, subkey, force)| k == key && *subkey == 0 && *force)
        .count()
}

/// `n` index reads of subkey 0, each answering its stored value.
async fn read_changed_n(pool: &RecordPool, lease: LeaseId, n: usize) {
    for _ in 0..n {
        let values = pool.read_changed(lease, &[0]).await.unwrap();
        assert_eq!(values.len(), 1, "the copy we hold is still answered");
        assert_eq!(values[0].1.data, b"v0");
    }
}

#[tokio::test(start_paused = true)]
async fn an_inflated_inspect_seq_drives_one_get_not_one_per_inspect() {
    let (dht, pool, lease, key) = written().await;
    dht.claim(&key, 0, 9);
    read_changed_n(&pool, lease, 5).await;
    assert_eq!(
        fetches(&dht, &key),
        1,
        "one fetch for the unconfirmed claim"
    );
    // Past every window, the same claim is still never fetched again.
    tokio::time::advance(Duration::from_secs(4 * 3600)).await;
    read_changed_n(&pool, lease, 2).await;
    assert_eq!(fetches(&dht, &key), 1, "the same claim, never again");
}

#[tokio::test(start_paused = true)]
async fn a_watch_hint_still_force_reads_a_capped_subkey() {
    let (dht, pool, lease, key) = written().await;
    dht.claim(&key, 0, 9);
    read_changed_n(&pool, lease, 2).await;
    assert_eq!(fetches(&dht, &key), 1, "the claim is capped");
    for hinted in 1..=2 {
        let values = pool.read_hinted(lease, &[0]).await.unwrap();
        assert_eq!(values.len(), 1);
        assert_eq!(
            fetches(&dht, &key),
            1 + hinted,
            "each hint reads the network at once"
        );
    }
}

#[tokio::test(start_paused = true)]
async fn a_higher_claim_is_fetched_once_per_doubling_window() {
    let (dht, pool, lease, key) = written().await;
    dht.claim(&key, 0, 9);
    read_changed_n(&pool, lease, 1).await;
    dht.claim(&key, 0, 12);
    read_changed_n(&pool, lease, 1).await;
    assert_eq!(
        fetches(&dht, &key),
        1,
        "a higher claim waits out the window"
    );
    tokio::time::advance(REFETCH_BACKOFF_MIN).await;
    read_changed_n(&pool, lease, 2).await;
    assert_eq!(fetches(&dht, &key), 2, "then is fetched once");
    dht.claim(&key, 0, 15);
    tokio::time::advance(REFETCH_BACKOFF_MIN).await;
    read_changed_n(&pool, lease, 1).await;
    assert_eq!(fetches(&dht, &key), 2, "the window doubled to 120 s");
    tokio::time::advance(REFETCH_BACKOFF_MIN).await;
    read_changed_n(&pool, lease, 1).await;
    assert_eq!(fetches(&dht, &key), 3);
}

#[tokio::test(start_paused = true)]
async fn a_value_at_the_claim_clears_it() {
    let (dht, pool, lease, key) = written().await;
    dht.claim(&key, 0, 1);
    read_changed_n(&pool, lease, 1).await;
    assert_eq!(fetches(&dht, &key), 1);
    // The value the claim named arrives (seq 1): read locally, it clears it.
    pool.set(lease, 0, b"v1".to_vec(), None).await.unwrap();
    let values = pool.read_changed(lease, &[0]).await.unwrap();
    assert_eq!(values[0].1.seq, 1);
    // So the next claim is fetched at once, inside the old window.
    dht.claim(&key, 0, 2);
    pool.read_changed(lease, &[0]).await.unwrap();
    assert_eq!(fetches(&dht, &key), 2, "a cleared claim holds nothing back");
}
