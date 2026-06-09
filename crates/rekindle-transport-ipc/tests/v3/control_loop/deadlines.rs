//! Deadline enforcement tests.
//!
//! The control loop checks three deadlines every iteration:
//! - Quiescence deadline: session was quiesced too long
//! - Rotation deadline: key rotation phase 2 didn't complete
//! - Drain deadline: graceful shutdown peer didn't finish draining
//!
//! Each deadline is set by its respective handler and checked by the
//! control loop. If the deadline is in the past, the control loop
//! returns the corresponding terminal SessionOutcome.
//!
//! These tests verify the deadline state on SessionContext — the
//! control loop's check is: if ctx.X_deadline().map_or(false, |d| Instant::now() >= d)
//! which is a trivial comparison. The important property is that
//! deadlines are set, cleared, and readable correctly.

use std::time::{Duration, Instant};

use rekindle_transport_ipc::v3::dispatch::test_helpers::make_test_context;

/// Quiescence deadline starts as None.
#[test]
fn quiescence_deadline_initially_none() {
    let (ctx, _) = make_test_context();
    assert!(ctx.quiescence_deadline().is_none());
}

/// Set quiescence deadline, verify it's readable.
#[test]
fn quiescence_deadline_set_and_readable() {
    let (mut ctx, _) = make_test_context();
    let deadline = Instant::now() + Duration::from_secs(60);
    ctx.set_quiescence_deadline(deadline);
    assert_eq!(ctx.quiescence_deadline(), Some(deadline));
}

/// Clear quiescence deadline after RESUME.
#[test]
fn quiescence_deadline_cleared() {
    let (mut ctx, _) = make_test_context();
    ctx.set_quiescence_deadline(Instant::now() + Duration::from_secs(60));
    assert!(ctx.quiescence_deadline().is_some());

    ctx.clear_quiescence_deadline();
    assert!(
        ctx.quiescence_deadline().is_none(),
        "cleared deadline must be None"
    );
}

/// Quiescence deadline in the past is detectable.
/// The control loop checks: Instant::now() >= deadline.
#[test]
fn quiescence_deadline_in_past_is_expired() {
    let (mut ctx, _) = make_test_context();
    let past = Instant::now() - Duration::from_secs(1);
    ctx.set_quiescence_deadline(past);

    let deadline = ctx.quiescence_deadline().unwrap();
    assert!(
        Instant::now() >= deadline,
        "past deadline must be detected as expired"
    );
}

/// Rotation deadline starts as None.
#[test]
fn rotation_deadline_initially_none() {
    let (ctx, _) = make_test_context();
    assert!(ctx.rotation_deadline().is_none());
}

/// Set and read rotation deadline.
#[test]
fn rotation_deadline_set_and_readable() {
    let (mut ctx, _) = make_test_context();
    let deadline = Instant::now() + Duration::from_secs(30);
    ctx.set_rotation_deadline(deadline);
    assert_eq!(ctx.rotation_deadline(), Some(deadline));
}

/// Clear rotation deadline after ROTATE_COMMIT.
#[test]
fn rotation_deadline_cleared() {
    let (mut ctx, _) = make_test_context();
    ctx.set_rotation_deadline(Instant::now() + Duration::from_secs(30));
    ctx.clear_rotation_deadline();
    assert!(ctx.rotation_deadline().is_none());
}

/// Rotation deadline in the past is expired.
#[test]
fn rotation_deadline_in_past_is_expired() {
    let (mut ctx, _) = make_test_context();
    let past = Instant::now() - Duration::from_millis(100);
    ctx.set_rotation_deadline(past);

    let deadline = ctx.rotation_deadline().unwrap();
    assert!(Instant::now() >= deadline);
}

/// Drain deadline starts as None.
#[test]
fn drain_deadline_initially_none() {
    let (ctx, _) = make_test_context();
    assert!(ctx.drain_deadline().is_none());
}

/// Set and read drain deadline.
#[test]
fn drain_deadline_set_and_readable() {
    let (mut ctx, _) = make_test_context();
    let deadline = Instant::now() + Duration::from_secs(5);
    ctx.set_drain_deadline(deadline);
    assert_eq!(ctx.drain_deadline(), Some(deadline));
}

/// Drain deadline in the past is expired.
#[test]
fn drain_deadline_in_past_is_expired() {
    let (mut ctx, _) = make_test_context();
    let past = Instant::now() - Duration::from_millis(50);
    ctx.set_drain_deadline(past);

    let deadline = ctx.drain_deadline().unwrap();
    assert!(Instant::now() >= deadline);
}

/// Multiple deadlines can coexist — the control loop checks all three
/// every iteration and returns the outcome for whichever fired.
#[test]
fn multiple_deadlines_coexist() {
    let (mut ctx, _) = make_test_context();
    let q = Instant::now() + Duration::from_secs(60);
    let r = Instant::now() + Duration::from_secs(30);
    let d = Instant::now() + Duration::from_secs(5);

    ctx.set_quiescence_deadline(q);
    ctx.set_rotation_deadline(r);
    ctx.set_drain_deadline(d);

    assert_eq!(ctx.quiescence_deadline(), Some(q));
    assert_eq!(ctx.rotation_deadline(), Some(r));
    assert_eq!(ctx.drain_deadline(), Some(d));
}

/// Clearing one deadline does not affect others.
#[test]
fn clearing_one_deadline_preserves_others() {
    let (mut ctx, _) = make_test_context();
    let q = Instant::now() + Duration::from_secs(60);
    let r = Instant::now() + Duration::from_secs(30);

    ctx.set_quiescence_deadline(q);
    ctx.set_rotation_deadline(r);

    ctx.clear_quiescence_deadline();

    assert!(ctx.quiescence_deadline().is_none());
    assert_eq!(
        ctx.rotation_deadline(),
        Some(r),
        "clearing quiescence must not affect rotation"
    );
}
