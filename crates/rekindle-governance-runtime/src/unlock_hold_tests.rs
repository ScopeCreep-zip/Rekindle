use std::sync::Arc;
use std::time::Duration;

use rekindle_types::display_catch_up::{PendingGovKind, PendingGovTarget};

use super::*;
use crate::deps::MockGovernanceRuntimeDeps;

fn invite(id: String) -> PendingGovOp {
    PendingGovOp::held_as(id, PendingGovKind::Kick, PendingGovTarget::default())
}

/// Deps whose barrier for every community is `barrier`, and whose held
/// writes run on the test's runtime.
fn deps_over(barrier: &Arc<UnlockBarrier>) -> MockGovernanceRuntimeDeps {
    let mut deps = MockGovernanceRuntimeDeps::new();
    let shared = Arc::clone(barrier);
    deps.expect_unlock_barrier()
        .returning(move |_| Arc::clone(&shared));
    deps.expect_emit_catch_up().returning(|_, _| ());
    deps.expect_spawn_held_write().returning(|_, write| {
        tokio::spawn(write);
    });
    deps
}

fn passed_with_unlanded_op() -> Arc<UnlockBarrier> {
    let barrier = Arc::new(UnlockBarrier::default());
    barrier.complete(Some(BarrierPhase::Passed));
    barrier.signed("aa");
    barrier
}

fn landing_within(bound: Duration) -> OpLanding {
    OpLanding::InviteLanded {
        invite_id: [7; 16],
        deadline: tokio::time::Instant::now() + bound,
    }
}

/// Plan E3.2 S7d-keys.8 (lead ruling 2026-11-24): past the barrier, an
/// invite link that may not be held waits in line behind our unlanded op
/// (listed while it waits) and signs once it landed, never "catching up".
#[tokio::test]
async fn an_invite_past_the_barrier_waits_in_line_for_our_earlier_op() {
    let barrier = passed_with_unlanded_op();
    let deps = deps_over(&barrier);
    let slot = tokio::spawn(async move {
        let slot = take_slot(&deps, "c1", landing_within(Duration::from_secs(60)), invite).await;
        (
            slot.map(|s| matches!(s, BarrierSlot::Now(t) if t.held.is_some())),
            deps,
        )
    });
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
    let listed = barrier.view().expect("the waiting write is listed").pending;
    assert_eq!(listed.len(), 1);
    barrier.settled("aa");
    let (slot, _deps) = slot.await.expect("the write task");
    assert!(matches!(slot, Ok(true)), "its turn came: {slot:?}");
}

/// The wait is bounded by the command's deadline: then a true reason
/// ("still being published"), the write unlisted and unsigned, and its
/// place in line passed on once its turn comes.
#[tokio::test(start_paused = true)]
async fn an_invite_whose_turn_does_not_come_by_its_deadline_is_refused_as_pending() {
    let barrier = Arc::new(UnlockBarrier::default());
    barrier.complete(Some(BarrierPhase::Passed));
    let (first_id, first) = barrier.hold_next(invite);
    let deps = deps_over(&barrier);
    let slot = take_slot(&deps, "c1", landing_within(Duration::from_secs(5)), invite).await;
    assert!(
        matches!(slot, Err(GovernanceRuntimeError::OwnOpsPending(ref c)) if c == "c1"),
        "{:?}",
        slot.err()
    );
    let listed = barrier.view().expect("the earlier write is listed").pending;
    assert_eq!(
        listed
            .iter()
            .map(|op| op.op_id.as_str())
            .collect::<Vec<_>>(),
        [first_id.as_str()]
    );
    barrier.serve(first).await;
    barrier.served(first);
    let (_, next) = barrier.hold_next(invite);
    tokio::time::timeout(Duration::from_secs(1), barrier.serve(next))
        .await
        .expect("the refused write's place passed on");
}

/// While the barrier catches up, the invite is still refused at once.
#[tokio::test]
async fn an_invite_is_refused_at_once_while_the_barrier_catches_up() {
    let barrier = Arc::new(UnlockBarrier::default());
    let deps = deps_over(&barrier);
    let slot = take_slot(&deps, "c1", landing_within(Duration::from_secs(60)), invite).await;
    assert!(matches!(slot, Err(GovernanceRuntimeError::CatchingUp(_))));
    assert!(barrier.view().is_some_and(|v| v.pending.is_empty()));
}
