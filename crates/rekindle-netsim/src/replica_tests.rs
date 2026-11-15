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
    // Five nodes: alone, node 0 reaches two replicas, fewer than the
    // `get_value_count` 3 a set that misses consensus still lands at
    // (0.5.8 `storage_manager/mod.rs:990-1030`).
    let (sim, nodes) = Netsim::nodes(5, 41);
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
        b: vec![1, 2, 3, 4],
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
    assert!((1..3).contains(&partial), "{partial} replicas took it");

    sim.fault(Fault::Heal);
    // The retry goes out at seq 1 again, a fresh encryption, and meets the
    // partial value; the tally settles it (0.5.8 `value_tally.rs:90-106`):
    // a partial holder answering first beats ours (nobody holds ours yet),
    // else ours gathers more holders; the seed fixes which.
    let at_one = |data: &[u8]| {
        sim.replica_values(&key, 0)
            .into_iter()
            .flatten()
            .filter(|v| v.seq == 1 && v.data == data)
            .count()
    };
    match pool.set(lease, 0, b"next".to_vec(), None).await.unwrap() {
        // A partial holder answered first: the fanout restarted on it.
        SetOutcome::Superseded(newer) => {
            assert_eq!((newer.seq, newer.data.as_slice()), (1, &b"partial"[..]));
            assert_eq!(
                pool.get(lease, 0, false).await.unwrap().unwrap().data,
                b"partial"
            );
        }
        // Ours settled over the partial rival: `Some(ours)`, which the pool
        // reads as landed (0.5.8 `set_value.rs:753-760`).
        SetOutcome::Landed => {
            assert!(at_one(b"next") >= 5 && at_one(b"partial") > 0);
        }
        other => panic!("unexpected {other:?}"),
    }
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
    // Five nodes, as in `a_partial_set_comes_back_as_our_own_stale_value`.
    let (sim, nodes) = Netsim::nodes(5, 43);
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
        b: vec![1, 2, 3, 4],
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

/// RV4 L3: a set that only `k` replicas accept (`Fault::WritePartial`) is
/// `BelowConsensus` with nothing stored locally (`AllowOffline(false)`,
/// `local_record_store_interface.rs:60-63`), the `k` closest replicas hold
/// it, so another node's get already sees it, and once the fault lifts the
/// same value re-sent lands on them unchanged (`set_value.rs:700-719`).
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_set_two_replicas_accept_is_below_consensus_yet_readable() {
    let (sim, nodes) = Netsim::nodes(3, 43);
    let pool = nodes[0].ports.records.start_session().unwrap();
    let (lease, key, _) = pool
        .create(&RecordSchema::Dflt { owner_subkeys: 2 }, None)
        .await
        .unwrap();
    assert_eq!(
        pool.set(lease, 0, b"v0".to_vec(), None).await.unwrap(),
        SetOutcome::Landed
    );
    sim.fault(Fault::WritePartial {
        node: 0,
        of: Writes::Subkey {
            subkey: 1,
            record: Some(crate::Records::CreatedDflt(2)),
        },
        accepting: 2,
    });
    assert_eq!(
        pool.set(lease, 1, b"op".to_vec(), None).await.unwrap(),
        SetOutcome::BelowConsensus
    );
    // Subkey 0 is outside the filter and still lands.
    assert_eq!(
        pool.set(lease, 0, b"v1".to_vec(), None).await.unwrap(),
        SetOutcome::Landed
    );
    let local = pool
        .inspect(lease, Some(&[1u32][..]), ReportScope::Local)
        .await
        .unwrap();
    assert_eq!(local.local_seqs, vec![None], "nothing stored locally");
    let holders: Vec<usize> = sim
        .replica_values(&key, 1)
        .iter()
        .enumerate()
        .filter(|(_, v)| v.as_ref().is_some_and(|v| v.data == b"op"))
        .map(|(r, _)| r)
        .collect();
    assert_eq!(holders, vec![0, 1], "the two closest replicas took it");

    let reader = nodes[1].ports.records.start_session().unwrap();
    let opened = reader.acquire(&key, None).await.unwrap();
    assert_eq!(
        reader.get(opened, 1, true).await.unwrap().map(|v| v.data),
        Some(b"op".to_vec()),
        "a reader reaching an accepting replica sees the value"
    );

    sim.fault(Fault::Heal);
    assert_eq!(
        pool.set(lease, 1, b"op".to_vec(), None).await.unwrap(),
        SetOutcome::Landed,
        "the retry meets its own value and lands"
    );
    let held = sim
        .replica_values(&key, 1)
        .into_iter()
        .flatten()
        .filter(|v| v.data == b"op" && v.seq == 0)
        .count();
    assert!(held >= 5, "one value at one seq: {held}");
}
