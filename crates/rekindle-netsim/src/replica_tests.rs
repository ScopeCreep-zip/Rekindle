//! Replicas, consensus and partial sets against veilid-core (plan E2.B S4:
//! M6, H1, H2; `records/replicas.rs` cites the storage manager).

use rekindle_records::port::{RecordSchema, ReportScope, SetOutcome};

use super::*;

/// H1: a set below consensus is `BelowConsensus` with the local seq not
/// moved, the replicas that took it keep it, and the next set (another
/// value at the same seq) comes back superseded by that partial value
/// (`set_value.rs:170-171,205-221,487-494,709-719`).
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_partial_set_comes_back_as_our_own_stale_value() {
    let (sim, nodes) = Netsim::nodes(3, 41);
    let pool = nodes[0].ports.records.start_session().unwrap();
    let (lease, key, _) = pool
        .create(&RecordSchema::Dflt { owner_subkeys: 1 }, None)
        .await
        .unwrap();
    assert_eq!(
        pool.set(lease, 0, b"v0".to_vec(), None).await.unwrap(),
        SetOutcome::Landed
    );

    // Alone, node 0 reaches only the replicas beside it: fewer than
    // `set_value_count`.
    sim.fault(Fault::Partition {
        a: vec![0],
        b: vec![1, 2],
    });
    assert_eq!(
        pool.set(lease, 0, b"partial".to_vec(), None).await.unwrap(),
        SetOutcome::BelowConsensus
    );
    let local = pool
        .inspect(lease, Some(&[0u32][..]), ReportScope::Local)
        .await
        .unwrap();
    assert_eq!(local.local_seqs, vec![Some(0)]);
    let partial = sim
        .replica_values(&key, 0)
        .into_iter()
        .flatten()
        .filter(|v| v.data == b"partial")
        .count();
    assert!((1..5).contains(&partial), "{partial} replicas took it");

    sim.fault(Fault::Heal);
    let SetOutcome::Superseded(newer) = pool.set(lease, 0, b"next".to_vec(), None).await.unwrap()
    else {
        panic!("our own partial value supersedes the retry");
    };
    assert_eq!((newer.seq, newer.data.as_slice()), (1, &b"partial"[..]));
    assert_eq!(
        pool.get(lease, 0, false).await.unwrap().unwrap().data,
        b"partial"
    );
}

/// H2: re-pushing the local copy unchanged while a replica holds a newer
/// value returns that value (`set_value.rs:211-218,417-494`), now the local
/// copy, never `Unchanged`.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn an_identical_re_push_behind_the_network_is_superseded() {
    let (sim, nodes) = Netsim::nodes(2, 42);
    let pool = nodes[0].ports.records.start_session().unwrap();
    let (lease, key, owner) = pool
        .create(&RecordSchema::Dflt { owner_subkeys: 1 }, None)
        .await
        .unwrap();
    assert_eq!(
        pool.set(lease, 0, b"a".to_vec(), None).await.unwrap(),
        SetOutcome::Landed
    );
    sim.forge_write(&key, 0, b"b".to_vec(), &owner).unwrap();
    let SetOutcome::Superseded(newer) = pool.set(lease, 0, b"a".to_vec(), None).await.unwrap()
    else {
        panic!("the newer network value supersedes the re-push");
    };
    assert_eq!((newer.seq, newer.data.as_slice()), (1, &b"b"[..]));
    assert_eq!(pool.get(lease, 0, false).await.unwrap().unwrap().data, b"b");
}

/// M6: across a partition each side writes to the replicas it reaches; a
/// side that reaches consensus lands, and a node reads what its side holds.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_partition_splits_the_replicas() {
    let (sim, nodes) = Netsim::nodes(3, 43);
    let (a, b) = (
        nodes[0].ports.records.start_session().unwrap(),
        nodes[1].ports.records.start_session().unwrap(),
    );
    let (lease, key, _) = a
        .create(&RecordSchema::Dflt { owner_subkeys: 1 }, None)
        .await
        .unwrap();
    a.set(lease, 0, b"v0".to_vec(), None).await.unwrap();
    let theirs = b.acquire(&key, None).await.unwrap();
    assert_eq!(b.get(theirs, 0, true).await.unwrap().unwrap().seq, 0);

    sim.fault(Fault::Partition {
        a: vec![0],
        b: vec![1, 2],
    });
    assert_eq!(
        a.set(lease, 0, b"lone".to_vec(), None).await.unwrap(),
        SetOutcome::BelowConsensus
    );
    // Node 1's side reaches none of the replicas that took "lone".
    let seen = b.get(theirs, 0, true).await.unwrap().unwrap();
    let report = b
        .inspect(theirs, Some(&[0u32][..]), ReportScope::SyncGet)
        .await
        .unwrap();
    assert_eq!(seen.data, b"v0");
    assert_eq!(report.network_seqs, vec![Some(0)]);
    let ours = a
        .inspect(lease, Some(&[0u32][..]), ReportScope::SyncGet)
        .await
        .unwrap();
    assert_eq!(
        (ours.local_seqs, ours.network_seqs),
        (vec![Some(0)], vec![Some(1)])
    );
}
