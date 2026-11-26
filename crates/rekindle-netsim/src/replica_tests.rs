//! Replicas, consensus and partial sets against veilid-core (plan E2.B S4:
//! M6, H1, H2; `records/replicas.rs` cites the storage manager).

use rekindle_records::port::{RecordSchema, ReportScope, SetOutcome};

use super::*;

/// H1: a set below consensus answers `Ok(None)` with nothing stored (fork
/// `e1158844` `set_value.rs:731-738` → `local_record_store_interface.rs:61-62`,
/// then `set_value.rs:306-308`), the pool's `Offline` since the local seq
/// did not move; the replicas that took
/// it keep it, and the next set (another value at the same seq) comes back
/// superseded by that partial value
/// (`set_value.rs:224-241,561-623,753-757`).
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_partial_set_comes_back_as_our_own_stale_value() {
    // Ten nodes: alone, node 0 reaches two of the 20 candidates, fewer than
    // the `get_value_count` 3 a set that misses consensus still lands at
    // (0.5.8 `storage_manager/mod.rs:990-1030`). An unreached candidate takes
    // no window slot (`fanout_call.rs:399-405`), so every candidate beside
    // node 0 is in its window.
    let (sim, nodes) = Netsim::nodes(10, 41);
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
        b: (1..10).collect(),
    });
    assert_eq!(
        pool.set(lease, 0, b"partial".to_vec(), None).await.unwrap(),
        SetOutcome::Offline
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

/// An identical re-push is `Unchanged` whether or not it reaches consensus
/// (DHT1, correcting VU2 item 15). Veilid re-sends the stored bytes at their
/// seq (0.5.8 `set_value.rs:353-366`) and, short of consensus with
/// `AllowOffline(false)`, answers `Ok(None)` (fork `e1158844`
/// `set_value.rs:731-738` → `local_record_store_interface.rs:61-62`, then
/// `:306-308`), as it does at consensus, and the local seq moves in
/// neither case. The value is already ours (it landed first), never a
/// missed new value; a new value below consensus is `Offline` (H1).
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn an_identical_re_push_below_consensus_stays_unchanged() {
    // Ten nodes, as in `a_partial_set_comes_back_as_our_own_stale_value`.
    let (sim, nodes) = Netsim::nodes(10, 44);
    let pool = nodes[0].ports.records.start_session().unwrap();
    let (lease, _, _) = pool
        .create(&RecordSchema::Dflt { owner_subkeys: 1 }, None)
        .await
        .unwrap();
    assert_eq!(
        pool.set(lease, 0, b"v0".to_vec(), None).await.unwrap(),
        SetOutcome::Landed
    );
    sim.fault(Fault::Partition {
        a: vec![0],
        b: (1..10).collect(),
    });
    assert_eq!(
        pool.set(lease, 0, b"v0".to_vec(), None).await.unwrap(),
        SetOutcome::Unchanged
    );
    sim.fault(Fault::Heal);
    assert_eq!(
        pool.set(lease, 0, b"v0".to_vec(), None).await.unwrap(),
        SetOutcome::Unchanged
    );
}

/// DHT1 (E2.5 durable `dht_write`): a durable write whose fanout ends below
/// consensus hears `Ok(None)` from Veilid with nothing stored (fork
/// `e1158844` `set_value.rs:306-308`); the pool reports it `Offline` by the
/// unmoved local seq, so `set_durable` holds it, and the re-push lands it
/// once the network takes it.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_durable_write_below_consensus_is_held_and_lands_on_the_re_push() {
    let (sim, nodes) = Netsim::nodes(3, 45);
    let pool = nodes[0].ports.records.start_session().unwrap();
    let (lease, key, _) = pool
        .create(&RecordSchema::Dflt { owner_subkeys: 2 }, None)
        .await
        .unwrap();
    let mut settled = pool.settled();
    sim.fault(Fault::WriteFails {
        node: 0,
        of: Writes::Subkey {
            subkey: 1,
            record: Some(crate::Records::CreatedDflt(2)),
        },
    });
    assert_eq!(
        pool.set_durable(lease, 1, b"op".to_vec(), None)
            .await
            .unwrap(),
        SetOutcome::Offline,
        "held, not landed"
    );
    assert_eq!(pool.held_writes(), vec![(key.clone(), 1)]);
    assert!(settled.try_recv().is_err(), "a miss settles nothing");
    let local = pool
        .inspect(lease, Some(&[1u32][..]), ReportScope::Local)
        .await
        .unwrap();
    assert_eq!(local.local_seqs, vec![None], "nothing stored locally");
    sim.fault(Fault::Heal);
    let event = tokio::time::timeout(std::time::Duration::from_secs(120), settled.recv())
        .await
        .expect("the re-push settles it")
        .unwrap();
    assert_eq!(
        (event.record_key, event.subkey, event.landed),
        (key.clone(), 1, true)
    );
    assert!(pool.held_writes().is_empty());
    assert_eq!(sim.network_get(&key, 1).unwrap().data, b"op");
}

/// H2: re-pushing the local copy unchanged while a replica holds a newer
/// value returns that value (0.5.8 `set_value.rs:353-366,561-623,753-757`), now the local
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
    // Ten nodes, as in `a_partial_set_comes_back_as_our_own_stale_value`.
    let (sim, nodes) = Netsim::nodes(10, 43);
    let (a, b) = (
        nodes[0].ports.records.start_session().unwrap(),
        nodes[1].ports.records.start_session().unwrap(),
    );
    let (lease, key, _) = a
        .create(&RecordSchema::Dflt { owner_subkeys: 1 }, None)
        .await
        .unwrap();
    a.set(lease, 0, b"v0".to_vec(), None).await.unwrap();
    let theirs = b
        .acquire(&key, None, rekindle_records::port::OpenIntent::Hold)
        .await
        .unwrap();
    assert_eq!(b.get(theirs, 0, true).await.unwrap().unwrap().seq, 0);

    sim.fault(Fault::Partition {
        a: vec![0],
        b: (1..10).collect(),
    });
    assert_eq!(
        a.set(lease, 0, b"lone".to_vec(), None).await.unwrap(),
        SetOutcome::Offline
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
/// `Offline` (Veilid's `Ok(None)`, the local seq not moved) with nothing stored locally (`AllowOffline(false)`,
/// 0.5.8 `local_record_store_interface.rs:60-63`), the `k` closest replicas hold
/// it, so another node's get already sees it. Once the fault lifts, the same
/// value re-sent has no local copy to resend, so it is a fresh encryption, a
/// rival of the bytes the two replicas hold (`record_encryption.rs:43`,
/// `value_tally.rs:53-57`): ours settles conflicted (`set_value.rs:753-760`)
/// and the pool writes it once above that seq (`set_value.rs:383-386`), so
/// every consensus replica ends on one value at one seq, the newest held.
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
        SetOutcome::Offline
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
    let opened = reader
        .acquire(&key, None, rekindle_records::port::OpenIntent::Hold)
        .await
        .unwrap();
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
    let values: Vec<_> = sim.replica_values(&key, 1).into_iter().flatten().collect();
    let newest = values.iter().map(|v| v.seq).max();
    assert_eq!(newest, Some(1), "written once above the contested seq 0");
    let at_newest: Vec<_> = values.iter().filter(|v| Some(v.seq) == newest).collect();
    assert!(
        at_newest.iter().all(|v| v.data == b"op"),
        "no rival at the newest seq"
    );
    assert!(
        at_newest.len() >= 5,
        "one value at one seq: {}",
        at_newest.len()
    );
}

/// A modified client's `min_seqnum` (plan E3.2 S9): a set raises the seq to
/// at least it after the automatic increment (0.5.8 `set_value.rs:384-386`,
/// `tests_storage_manager/test_min_seqnum.rs:83-104`), skipping seqs; a
/// minimum the increment already passes changes nothing, and other nodes'
/// sets are unaffected.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_min_seqnum_set_skips_seqs() {
    let (sim, nodes) = Netsim::nodes(4, 43);
    let pool = nodes[0].ports.records.start_session().unwrap();
    let (lease, key, _) = pool
        .create(&RecordSchema::Dflt { owner_subkeys: 1 }, None)
        .await
        .unwrap();
    let seq_after = |data: &'static [u8]| {
        let pool = pool.clone();
        async move {
            assert_eq!(
                pool.set(lease, 0, data.to_vec(), None).await.unwrap(),
                SetOutcome::Landed
            );
            let local = pool.inspect(lease, Some(&[0u32][..]), ReportScope::Local);
            local.await.unwrap().local_seqs[0]
        }
    };
    assert_eq!(seq_after(b"v0").await, Some(0));
    sim.fault(Fault::MinSeqnum {
        node: 0,
        key: key.clone(),
        min_seqnum: Some(5),
    });
    assert_eq!(seq_after(b"v5").await, Some(5));
    assert_eq!(seq_after(b"v6").await, Some(6), "5 is already passed");
    sim.fault(Fault::MinSeqnum {
        node: 0,
        key: key.clone(),
        min_seqnum: None,
    });
    assert_eq!(seq_after(b"v7").await, Some(7));
    let held = sim.replica_values(&key, 0).into_iter().flatten();
    assert!(held.into_iter().any(|v| v.seq == 7 && v.data == b"v7"));
}

/// While a node's pool routes fail, none of its DHT questions reaches a
/// replica, since each rides the record's safety selection over a pool
/// safety route ([`super::records::reaches`] cites the storage manager):
/// its set lands nowhere and answers `Offline`, and its open of a record it
/// does not hold finds no node to answer. Another node's DHT is untouched,
/// and once the routes carry traffic again the set lands.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_node_whose_pool_routes_fail_reaches_no_replica() {
    let (sim, nodes) = Netsim::nodes(4, 45);
    let pool = nodes[0].ports.records.start_session().unwrap();
    let (lease, key, _) = pool
        .create(&RecordSchema::Dflt { owner_subkeys: 1 }, None)
        .await
        .unwrap();
    assert_eq!(
        pool.set(lease, 0, b"v0".to_vec(), None).await.unwrap(),
        SetOutcome::Landed
    );
    let other = nodes[1].ports.records.start_session().unwrap();
    let (other_lease, other_key, _) = other
        .create(&RecordSchema::Dflt { owner_subkeys: 1 }, None)
        .await
        .unwrap();
    assert_eq!(
        other
            .set(other_lease, 0, b"o0".to_vec(), None)
            .await
            .unwrap(),
        SetOutcome::Landed
    );

    sim.fault(Fault::PoolRoutesFail {
        node: 0,
        failing: true,
    });
    assert_eq!(
        pool.set(lease, 0, b"v1".to_vec(), None).await.unwrap(),
        SetOutcome::Offline
    );
    let landed = sim
        .replica_values(&key, 0)
        .into_iter()
        .flatten()
        .filter(|v| v.data == b"v1")
        .count();
    assert_eq!(landed, 0, "no replica took the failing node's set");
    assert!(
        pool.acquire(&other_key, None, rekindle_records::port::OpenIntent::Hold)
            .await
            .is_err(),
        "the failing node's open of a record it does not hold finds no node"
    );
    // Node 1's DHT is untouched: it opens node 0's record and reads v0.
    let read = other
        .acquire(&key, None, rekindle_records::port::OpenIntent::Hold)
        .await
        .unwrap();
    assert_eq!(other.get(read, 0, true).await.unwrap().unwrap().data, b"v0");

    sim.fault(Fault::PoolRoutesFail {
        node: 0,
        failing: false,
    });
    assert_eq!(
        pool.set(lease, 0, b"v1".to_vec(), None).await.unwrap(),
        SetOutcome::Landed
    );
}
