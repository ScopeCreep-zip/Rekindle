use std::time::Duration;

use rekindle_types::display_catch_up::{
    PendingGovKind, PendingGovOp, PendingGovState, PendingGovTarget,
};

use super::*;

fn round(
    local: Option<u64>,
    dht: Option<u64>,
    ring: bool,
    answers: &[Option<u64>],
) -> BarrierRound {
    BarrierRound {
        local_head: local,
        dht_head: dht,
        ring_known: ring,
        answers: answers.to_vec(),
    }
}

fn held(id: &str) -> PendingGovOp {
    PendingGovOp {
        op_id: id.into(),
        kind: PendingGovKind::Kick,
        target: PendingGovTarget::default(),
        label: "Kick Bea".into(),
        state: PendingGovState::CatchingUp,
    }
}

/// Plan S7d-keys.8, N5: "one stale peer reporting lower does not lower the
/// barrier"; the result is the maximum over every answer and the DHT head.
#[test]
fn the_barrier_takes_the_highest_head_over_every_answering_peer() {
    assert_eq!(
        judge_round(&round(Some(7), Some(5), true, &[Some(2), Some(7), None])),
        Some(BarrierPhase::Passed)
    );
    assert_eq!(
        judge_round(&round(Some(7), Some(5), true, &[Some(2), Some(9)])),
        Some(BarrierPhase::Restored(RestoreDetected {
            last_seq: 9,
            local_head: Some(7)
        }))
    );
}

/// IC3: a round in which no known ring peer answered is retried, never
/// completed on the DHT read alone, however high the DHT head reads.
#[test]
fn the_barrier_retries_until_a_known_ring_peer_answers() {
    let barrier = UnlockBarrier::default();
    barrier.complete(judge_round(&round(Some(3), Some(3), true, &[])));
    assert_eq!(barrier.phase(), BarrierPhase::CatchingUp);
    barrier.complete(judge_round(&round(Some(3), Some(3), true, &[None])));
    assert_eq!(barrier.phase(), BarrierPhase::Passed);
}

/// N5: a sole creator (no ring peer known) passes on its own DHT head.
#[test]
fn a_single_member_community_creator_passes_the_barrier() {
    assert_eq!(
        judge_round(&round(Some(4), Some(4), false, &[])),
        Some(BarrierPhase::Passed)
    );
    assert_eq!(
        judge_round(&round(None, None, false, &[])),
        Some(BarrierPhase::Passed)
    );
}

/// IC4: the restore's `lastSeq` is the barrier's maximum (a peer's walk
/// above the DHT head counts: the old copy gossiped it before its append
/// landed), and a restore is final for the unlock.
#[test]
fn a_restore_retires_at_the_barrier_maximum_not_the_dht_head() {
    let barrier = UnlockBarrier::default();
    barrier.complete(judge_round(&round(Some(2), Some(5), true, &[Some(6)])));
    let fact = RestoreDetected {
        last_seq: 6,
        local_head: Some(2),
    };
    assert_eq!(barrier.phase(), BarrierPhase::Restored(fact));
    barrier.complete(Some(BarrierPhase::Passed));
    assert_eq!(
        barrier.phase(),
        BarrierPhase::Restored(fact),
        "a restore stays"
    );
}

/// "Governance writes are held in the outbox as 'pending: catching up'
/// until every result reads at or below the local head": a write waits,
/// is listed, and goes ahead only when a round completes.
#[tokio::test(start_paused = true)]
async fn governance_writes_wait_for_the_unlock_barrier() {
    let barriers = UnlockBarriers::default();
    let barrier = barriers.of("c1");
    barrier.hold(held("h1"));
    let view = barriers.view("c1").expect("a held write is shown");
    assert!(view.catching_up);
    assert_eq!(view.pending[0].state, PendingGovState::CatchingUp);

    let waiter = {
        let barrier = Arc::clone(&barrier);
        tokio::spawn(async move { barrier.passed().await })
    };
    tokio::time::sleep(Duration::from_secs(60)).await;
    assert!(!waiter.is_finished(), "no round completed: the write waits");

    barrier.complete(judge_round(&round(Some(1), Some(1), true, &[Some(1)])));
    assert_eq!(waiter.await.unwrap(), Ok(()));
    barrier.signing("h1");
    assert_eq!(
        barriers.view("c1").unwrap().pending[0].state,
        PendingGovState::Signing
    );
    barrier.release("h1");
    assert_eq!(barriers.view("c1"), None, "cleared once passed and empty");
}

/// A restored copy never signs: the wait answers the restore fact.
#[tokio::test]
async fn a_restored_barrier_refuses_to_sign() {
    let barrier = UnlockBarrier::default();
    barrier.complete(judge_round(&round(None, Some(3), false, &[])));
    assert_eq!(
        barrier.passed().await,
        Err(RestoreDetected {
            last_seq: 3,
            local_head: None
        })
    );
}

#[test]
fn highest_seq_counts_only_our_device() {
    let head = |device: [u8; 32], seq: u64| rekindle_types::governance::OpRef {
        op_hash: rekindle_types::governance::OpHash([u8::try_from(seq).unwrap(); 32]),
        author: rekindle_types::id::PseudonymKey([9; 32]),
        author_record: String::new(),
        author_seq: seq,
        device_key: device,
    };
    let frontier = Frontier {
        heads: vec![head([1; 32], 4), head([2; 32], 9), head([1; 32], 6)],
        roster_checksum: rekindle_types::governance::RosterChecksum([0; 32]),
        fork_refs: Vec::new(),
        fork_proof: Vec::new(),
    };
    assert_eq!(highest_seq_of(&frontier, &[1; 32]), Some(6));
    assert_eq!(highest_seq_of(&frontier, &[3; 32]), None);
}

/// Seq n + 1 waits for seq n's append (S7d-keys.8): `landed` holds while
/// an op we signed has not landed, and returns once it settled.
#[tokio::test]
async fn the_next_write_waits_until_our_signed_op_landed() {
    let barrier = std::sync::Arc::new(UnlockBarrier::default());
    barrier.signed("aa");
    let waiting = {
        let barrier = std::sync::Arc::clone(&barrier);
        tokio::spawn(async move { barrier.landed().await })
    };
    tokio::task::yield_now().await;
    assert!(
        !waiting.is_finished(),
        "an unlanded op holds the next write"
    );
    barrier.settled("bb");
    tokio::task::yield_now().await;
    assert!(
        !waiting.is_finished(),
        "another op settling does not release it"
    );
    barrier.settled("aa");
    waiting.await.unwrap();
    assert_ne!(barrier.next_op_id(), barrier.next_op_id());
}

/// Lead decision 1 (plan E3.2 S7d-keys.8): a copy that holds its own key's
/// later ops, echoed back by the unlock sweep, but whose `signed_here` head
/// is behind the DHT `GovHead` is a restore.
#[tokio::test]
async fn a_copy_holding_its_own_echoed_ops_but_behind_its_signed_head_is_a_restore() {
    use crate::op_log::{HeldOp, OpFate};
    let device = [7u8; 32];
    let echoed: Vec<HeldOp> = (0..=5)
        .map(|seq| HeldOp {
            hash: rekindle_types::governance::OpHash([u8::try_from(seq).unwrap(); 32]),
            device_key: device,
            author: rekindle_types::id::PseudonymKey([1; 32]),
            author_record: "VLD0:m".into(),
            seq,
            lamport: seq,
            issued_at: 0,
            encoded: Vec::new(),
            fate: OpFate::Applied,
        })
        .collect();
    let mut store = crate::deps::MockGovernanceOpStore::new();
    store
        .expect_governance_ops()
        .returning(move |_| Ok(echoed.clone()));
    store
        .expect_signed_here_head()
        .returning(|_, _| Ok(Some(2)));
    let local = local_head(&store, "c1", &device).await.unwrap();
    let verdict = judge_round(&round(local, Some(5), true, &[Some(5)]));
    assert_eq!(
        verdict,
        Some(BarrierPhase::Restored(RestoreDetected {
            last_seq: 5,
            local_head: Some(2),
        })),
        "ops echoed back never stand for what this copy signed"
    );
}

/// A write signs at once only once the barrier passed with nothing held;
/// held writes queue in arrival order (ticket n + 1 after ticket n).
#[tokio::test]
async fn held_writes_queue_in_arrival_order_behind_the_barrier() {
    let barrier = Arc::new(UnlockBarrier::default());
    assert!(barrier.try_turn_now().is_none(), "catching up: held");
    let (first, t0) = barrier.hold_next(|id| PendingGovOp {
        op_id: id,
        ..held("")
    });
    let (_, t1) = barrier.hold_next(|id| PendingGovOp {
        op_id: id,
        ..held("")
    });
    barrier.complete(Some(BarrierPhase::Passed));
    assert!(barrier.try_turn_now().is_none(), "a held write goes first");
    let second = {
        let barrier = Arc::clone(&barrier);
        tokio::spawn(async move { barrier.serve(t1).await })
    };
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(!second.is_finished(), "ticket 1 waits for ticket 0");
    barrier.serve(t0).await;
    barrier.served(t0);
    tokio::time::timeout(Duration::from_secs(1), second)
        .await
        .expect("ticket 1 queues once ticket 0 holds the turn")
        .unwrap();
    barrier.release(&first);
    barrier.release("held-1");
    assert!(barrier.try_turn_now().is_some(), "passed, nothing held");
}
