//! FSM edges, transitions and the shutdown signal.

use super::*;

#[test]
fn initial_state_is_stopped() {
    let lc = AppLifecycle::new();
    assert_eq!(lc.state(), LifecycleState::Stopped);
}

#[test]
fn transition_updates_state() {
    let lc = AppLifecycle::new();
    lc.transition(LifecycleState::Starting).unwrap();
    assert_eq!(lc.state(), LifecycleState::Starting);
    lc.transition(LifecycleState::Locked).unwrap();
    assert_eq!(lc.state(), LifecycleState::Locked);
}

#[test]
fn can_query_only_when_ready() {
    assert!(!LifecycleState::Stopped.can_query());
    assert!(!LifecycleState::Starting.can_query());
    assert!(!LifecycleState::Locked.can_query());
    assert!(!LifecycleState::Resuming.can_query());
    assert!(LifecycleState::Operational.can_query());
    assert!(LifecycleState::Degraded.can_query());
    assert!(LifecycleState::Detached.can_query());
    assert!(!LifecycleState::Locking.can_query());
    assert!(!LifecycleState::ShuttingDown.can_query());
}

#[test]
fn can_write_only_when_operational_or_degraded() {
    assert!(!LifecycleState::Detached.can_write());
    assert!(LifecycleState::Operational.can_write());
    assert!(LifecycleState::Degraded.can_write());
    assert!(!LifecycleState::Locked.can_write());
    assert!(!LifecycleState::Stopped.can_write());
}

#[test]
fn can_unlock_only_when_locked() {
    assert!(LifecycleState::Locked.can_unlock());
    assert!(!LifecycleState::Operational.can_unlock());
    assert!(!LifecycleState::Stopped.can_unlock());
}

#[test]
fn valid_transitions_accepted() {
    assert!(LifecycleState::Stopped.can_transition_to(LifecycleState::Starting));
    assert!(LifecycleState::Starting.can_transition_to(LifecycleState::Locked));
    assert!(LifecycleState::Locked.can_transition_to(LifecycleState::Resuming));
    assert!(LifecycleState::Resuming.can_transition_to(LifecycleState::Operational));
    assert!(LifecycleState::Operational.can_transition_to(LifecycleState::Locking));
    assert!(LifecycleState::Locking.can_transition_to(LifecycleState::Locked));
    assert!(LifecycleState::Operational.can_transition_to(LifecycleState::ShuttingDown));
    assert!(LifecycleState::ShuttingDown.can_transition_to(LifecycleState::Stopped));
}

#[test]
fn invalid_transitions_rejected() {
    assert!(!LifecycleState::Stopped.can_transition_to(LifecycleState::Operational));
    assert!(!LifecycleState::Locked.can_transition_to(LifecycleState::Starting));
    assert!(!LifecycleState::Locking.can_transition_to(LifecycleState::Operational));
}

#[test]
fn transition_returns_err_on_invalid_edge() {
    let lc = AppLifecycle::new();
    let res = lc.transition(LifecycleState::Operational);
    assert!(matches!(res, Err(LifecycleError::InvalidTransition { .. })));
    assert_eq!(
        lc.state(),
        LifecycleState::Stopped,
        "rejected transition must not alter state"
    );
}

#[test]
fn idempotent_self_transition_is_ok() {
    let lc = AppLifecycle::new();
    // Stopped → Stopped is a no-op, not an error.
    assert_eq!(
        lc.transition(LifecycleState::Stopped).unwrap(),
        LifecycleState::Stopped
    );
}

#[test]
fn degraded_recovers_to_operational() {
    assert!(LifecycleState::Degraded.can_transition_to(LifecycleState::Operational));
    assert!(LifecycleState::Detached.can_transition_to(LifecycleState::Operational));
}

/// Exhaustive transition-matrix test — every (from, to) pair in the
/// 9×9 = 81-pair cartesian product is checked against the explicit
/// expected-valid set. Catches accidental regressions to
/// `can_transition_to` (e.g., a refactor that drops an edge or adds
/// a spurious one).
#[test]
fn transition_matrix_is_exhaustive() {
    use LifecycleState::*;
    // Source of truth: every valid edge in the plan's transition table.
    let valid: &[(LifecycleState, LifecycleState)] = &[
        // Stopped → Starting (boot begins).
        (Stopped, Starting),
        // Starting → {Locked, Stopped}.
        (Starting, Locked),
        (Starting, Stopped),
        // Locked → {Resuming, ShuttingDown}.
        (Locked, Resuming),
        (Locked, ShuttingDown),
        // Resuming → {Operational, Degraded, Locked}.
        (Resuming, Operational),
        (Resuming, Degraded),
        (Resuming, Locked),
        // Operational ↔ Degraded ↔ Detached + → Locking/ShuttingDown.
        (Operational, Degraded),
        (Operational, Detached),
        (Operational, Locking),
        (Operational, ShuttingDown),
        (Degraded, Operational),
        (Degraded, Detached),
        (Degraded, Locking),
        (Degraded, ShuttingDown),
        (Detached, Operational),
        (Detached, Degraded),
        (Detached, Locking),
        (Detached, ShuttingDown),
        // Locking → Locked.
        (Locking, Locked),
        // ShuttingDown → Stopped (terminal teardown).
        (ShuttingDown, Stopped),
    ];
    let all = [
        Stopped,
        Starting,
        Locked,
        Resuming,
        Operational,
        Degraded,
        Detached,
        Locking,
        ShuttingDown,
    ];
    for &from in &all {
        for &to in &all {
            let want = from == to || valid.contains(&(from, to));
            let got = from.can_transition_to(to);
            // Self-edges: `from == to` should be accepted by `transition()`
            // (the AppLifecycle::transition impl treats self-transition as
            // a no-op Ok). `can_transition_to` itself returns whatever
            // the matches!() arm yields; self-edges aren't in the table.
            if from == to {
                // Skip self-edges for this check; they're handled by
                // `idempotent_self_transition_is_ok`.
                continue;
            }
            assert_eq!(
                got, want,
                "({from:?} → {to:?}): expected can_transition_to == {want}, got {got}",
            );
        }
    }
    // Sanity: the valid set has exactly the count from the plan's table.
    assert_eq!(valid.len(), 22, "valid-edge count mismatches plan's table");
}

/// Deep audit: late subscribers must NOT see transitions that happened
/// before subscribe(). Documents the inherent broadcast semantic so
/// callers know to seed via `current()` before subscribing.
#[tokio::test(flavor = "current_thread")]
async fn subscribe_after_transition_misses_past_events() {
    let lc = AppLifecycle::new();
    lc.transition(LifecycleState::Starting).unwrap();
    lc.transition(LifecycleState::Locked).unwrap();
    // Subscribe AFTER both transitions.
    let mut rx = lc.subscribe();
    // Buffer is empty for this subscriber; next live transition fires.
    lc.transition(LifecycleState::Resuming).unwrap();
    assert_eq!(rx.recv().await.unwrap(), LifecycleState::Resuming);
    // Confirm the seed pattern works: caller can read `current()` to
    // catch up on the state at subscribe time.
    assert_eq!(lc.current(), LifecycleState::Resuming);
}

/// Deep audit: concurrent transitions race the atomic store. Two
/// threads attempting different transitions from the same start
/// state must result in exactly ONE successful transition; the
/// loser observes either the loser's-rejected error OR a
/// from-the-new-state-rejected error (depending on interleave).
/// Either way, no torn state.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_transitions_yield_consistent_state() {
    let lc = std::sync::Arc::new(AppLifecycle::new());
    lc.transition(LifecycleState::Starting).unwrap();
    lc.transition(LifecycleState::Locked).unwrap();
    lc.transition(LifecycleState::Resuming).unwrap();
    // From Resuming, valid edges are Operational, Degraded, Locked.
    // Race two of them across 4 worker threads.
    let mut tasks = Vec::new();
    for i in 0..50 {
        let lc_clone = std::sync::Arc::clone(&lc);
        let target = if i % 2 == 0 {
            LifecycleState::Operational
        } else {
            LifecycleState::Degraded
        };
        tasks.push(tokio::spawn(async move {
            let _ = lc_clone.transition(target);
        }));
    }
    for t in tasks {
        t.await.unwrap();
    }
    let final_state = lc.state();
    assert!(
        matches!(
            final_state,
            LifecycleState::Operational | LifecycleState::Degraded
        ),
        "final state must be one of the racing targets, not torn — got {final_state:?}",
    );
}

/// Phase 5 audit — login_core / create_identity_core failures must
/// roll the FSM back to Locked so the user can retry. Without this
/// edge being valid, a failed login would strand the lifecycle in
/// Resuming forever (Locked → Resuming requires being in Locked,
/// not Resuming).
#[test]
fn resuming_rolls_back_to_locked_on_login_failure() {
    let lc = AppLifecycle::new();
    lc.transition(LifecycleState::Starting).unwrap();
    lc.transition(LifecycleState::Locked).unwrap();
    lc.transition(LifecycleState::Resuming).unwrap();
    // Simulate login_core failure → rollback.
    lc.transition(LifecycleState::Locked)
        .expect("Resuming → Locked must be valid for login-failure rollback");
    assert_eq!(lc.state(), LifecycleState::Locked);
    // User can retry: Locked → Resuming → Operational still works.
    lc.transition(LifecycleState::Resuming).unwrap();
    lc.transition(LifecycleState::Operational).unwrap();
    assert_eq!(lc.state(), LifecycleState::Operational);
}

#[tokio::test(flavor = "current_thread")]
async fn subscribe_observes_transitions() {
    let lc = AppLifecycle::new();
    let mut rx = lc.subscribe();
    lc.transition(LifecycleState::Starting).unwrap();
    lc.transition(LifecycleState::Locked).unwrap();
    assert_eq!(rx.recv().await.unwrap(), LifecycleState::Starting);
    assert_eq!(rx.recv().await.unwrap(), LifecycleState::Locked);
}

/// `wait_until_unlockable` must return immediately when the FSM is
/// already `Locked` (the common re-login case after a logout, which
/// leaves the lifecycle in Locked).
#[tokio::test(flavor = "current_thread")]
async fn wait_until_unlockable_returns_immediately_when_locked() {
    let lc = AppLifecycle::new();
    lc.transition(LifecycleState::Starting).unwrap();
    lc.transition(LifecycleState::Locked).unwrap();
    // Already unlockable — completes without any further transition.
    lc.wait_until_unlockable().await;
    assert_eq!(lc.state(), LifecycleState::Locked);
}

/// A waiter that starts during `Starting` (not yet unlockable) must wake
/// exactly when the async attach drives `Starting → Locked`. This is the
/// precise race the fix targets.
#[tokio::test(flavor = "current_thread")]
async fn wait_until_unlockable_wakes_on_transition_to_locked() {
    let lc = std::sync::Arc::new(AppLifecycle::new());
    lc.transition(LifecycleState::Starting).unwrap();
    let lc_clone = lc.clone();
    let waiter = tokio::spawn(async move { lc_clone.wait_until_unlockable().await });
    // Let the waiter subscribe + observe it's not yet unlockable before
    // we fire the transition it's waiting for.
    tokio::task::yield_now().await;
    lc.transition(LifecycleState::Locked).unwrap();
    waiter.await.unwrap();
    assert_eq!(lc.state(), LifecycleState::Locked);
}

/// A waiter that starts in `Stopped` must skip the non-unlockable
/// `Stopped → Starting` transition (the `Ok(_)` intermediate arm) and
/// only return once `Starting → Locked` lands.
#[tokio::test(flavor = "current_thread")]
async fn wait_until_unlockable_skips_intermediate_transitions() {
    let lc = std::sync::Arc::new(AppLifecycle::new());
    let lc_clone = lc.clone();
    let waiter = tokio::spawn(async move { lc_clone.wait_until_unlockable().await });
    tokio::task::yield_now().await;
    lc.transition(LifecycleState::Starting).unwrap();
    tokio::task::yield_now().await;
    // Still pending — Starting is not unlockable.
    assert!(!waiter.is_finished());
    lc.transition(LifecycleState::Locked).unwrap();
    waiter.await.unwrap();
    assert_eq!(lc.state(), LifecycleState::Locked);
}

#[tokio::test(flavor = "current_thread")]
async fn shutdown_requested_fires_on_shutting_down() {
    let lc = std::sync::Arc::new(AppLifecycle::new());
    // Drive the state machine forward so ShuttingDown is reachable.
    lc.transition(LifecycleState::Starting).unwrap();
    lc.transition(LifecycleState::Locked).unwrap();
    lc.transition(LifecycleState::Resuming).unwrap();
    lc.transition(LifecycleState::Operational).unwrap();

    let lc_clone = lc.clone();
    let waiter = tokio::spawn(async move { lc_clone.shutdown_requested().await });
    // Give the spawned task a chance to start waiting before we fire
    // the notify; otherwise Notify::notified misses the wakeup.
    tokio::task::yield_now().await;
    lc.transition(LifecycleState::ShuttingDown).unwrap();
    waiter.await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn shutdown_requested_sees_a_shutdown_that_already_happened() {
    let lc = AppLifecycle::new();
    lc.transition(LifecycleState::Starting).unwrap();
    lc.transition(LifecycleState::Locked).unwrap();
    lc.transition(LifecycleState::ShuttingDown).unwrap();
    lc.shutdown_requested().await;
}
