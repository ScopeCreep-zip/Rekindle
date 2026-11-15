//! How the pool reports a set whose result is a settled value (plan VU1,
//! decision 4): Veilid 0.5.8 returns `Some(v)` when the value differs from
//! ours *or* a node holds a rival at our seq (`set_value.rs:753-760`), so
//! `Some(ours)` is a stored write and only another value supersedes it.

use std::sync::Arc;
use std::time::Duration;

use rekindle_records::chain;
use rekindle_records::port::{RecordPort, RecordSchema, SetOutcome};

use crate::test_support::{pool_on, Contest, MemDht};

const DFLT_2: RecordSchema = RecordSchema::Dflt { owner_subkeys: 2 };

async fn created(pool: &crate::RecordPool) -> (rekindle_records::lease::LeaseId, String) {
    let (lease, key, _) = RecordPort::create(pool, &DFLT_2, None).await.unwrap();
    (lease, key)
}

#[tokio::test]
async fn a_set_that_settles_on_ours_over_a_rival_is_landed_then_unchanged() {
    let dht = Arc::new(MemDht::default());
    let pool = pool_on(Arc::clone(&dht));
    let (lease, _) = created(&pool).await;
    dht.contest(Contest::Ours);
    let first = pool.set(lease, 0, b"ours".to_vec(), None).await.unwrap();
    assert_eq!(first, SetOutcome::Landed, "ours settled at a new seq");
    let again = pool.set(lease, 0, b"ours".to_vec(), None).await.unwrap();
    assert_eq!(
        again,
        SetOutcome::Unchanged,
        "the identical rewrite kept its seq"
    );
    assert!(!first.missed() && !again.missed());
}

#[tokio::test]
async fn a_set_that_settles_on_another_value_is_superseded_by_it() {
    let dht = Arc::new(MemDht::default());
    let pool = pool_on(Arc::clone(&dht));
    let (lease, _) = created(&pool).await;
    dht.contest(Contest::Rival {
        data: b"theirs".to_vec(),
        writer: "another-device".into(),
    });
    let outcome = pool.set(lease, 0, b"ours".to_vec(), None).await.unwrap();
    let SetOutcome::Superseded(rival) = outcome else {
        panic!("expected Superseded, got {outcome:?}");
    };
    assert_eq!(rival.data, b"theirs");

    // Our data under another writer is not ours either.
    dht.contest(Contest::Rival {
        data: b"same bytes".to_vec(),
        writer: "another-device".into(),
    });
    let outcome = pool
        .set(lease, 1, b"same bytes".to_vec(), None)
        .await
        .unwrap();
    assert!(matches!(outcome, SetOutcome::Superseded(_)), "{outcome:?}");
}

/// `durable.rs`'s direct path: a contested landing settles as landed, so a
/// channel's pending messages are confirmed, not failed.
#[tokio::test]
async fn a_durable_write_that_settles_on_ours_is_settled_landed() {
    let dht = Arc::new(MemDht::default());
    let pool = pool_on(Arc::clone(&dht));
    let (lease, key) = created(&pool).await;
    let mut settled = pool.settled();
    dht.contest(Contest::Ours);
    let outcome = pool.set_durable(lease, 0, b"post".to_vec()).await.unwrap();
    assert_eq!(outcome, SetOutcome::Landed);
    let event = settled
        .try_recv()
        .expect("the direct path publishes the settle");
    assert_eq!(
        (event.record_key, event.subkey, event.landed),
        (key, 0, true)
    );
}

/// `durable.rs`'s re-push path: a write held while offline that later
/// settles on ours over a rival is settled landed.
#[tokio::test(start_paused = true)]
async fn a_held_write_re_pushed_onto_a_contested_subkey_is_settled_landed() {
    let dht = Arc::new(MemDht::default());
    let pool = pool_on(Arc::clone(&dht));
    let (lease, key) = created(&pool).await;
    let mut settled = pool.settled();
    dht.offline(true);
    let outcome = pool.set_durable(lease, 1, b"post".to_vec()).await.unwrap();
    assert_eq!(outcome, SetOutcome::Offline, "held for the re-push");
    dht.offline(false);
    dht.contest(Contest::Ours);
    let event = tokio::time::timeout(Duration::from_secs(120), settled.recv())
        .await
        .expect("the re-push settles it")
        .unwrap();
    assert_eq!(dht.value(&event.record_key, 1).unwrap().data, b"post");
    assert_eq!(
        (event.record_key, event.subkey, event.landed),
        (key, 1, true)
    );
}

/// `chain/mod.rs`: a DM-chain append whose sealed page and `HEAD` writes
/// settle on ours over a rival is stored (`require_stored`), not
/// `NotStored`.
#[tokio::test]
async fn a_chain_append_onto_contested_subkeys_is_stored() {
    let dht = Arc::new(MemDht::default());
    let pool = pool_on(Arc::clone(&dht));
    let (lease, key, owner) = RecordPort::create(&*pool, &DFLT_2, None).await.unwrap();
    pool.release(lease).await;
    dht.contest(Contest::Ours);
    let pointer = chain::append_at(&*pool, &key, 0, &owner, &[b"hello".to_vec()])
        .await
        .expect("the append is stored");
    assert_eq!((pointer.first_seq, pointer.last_seq), (1, 1));
    assert!(dht.value(&key, 0).is_some(), "HEAD names the page");
    assert!(
        dht.value(&pointer.page_key, 0).is_some(),
        "the page is written"
    );
}
