use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

/// Events recorded before install are drained in order on install.
#[test]
fn record_then_install_drains_in_order() {
    let buf = ColdStartBuffer::<u32>::new();
    assert!(buf.try_record(1).is_ok());
    assert!(buf.try_record(2).is_ok());
    assert!(buf.try_record(3).is_ok());
    assert_eq!(buf.buffered_count(), Some(3));

    let collected = Arc::new(parking_lot::Mutex::new(Vec::new()));
    let c = Arc::clone(&collected);
    buf.install_callback(move |ev| c.lock().push(ev));
    let got = collected.lock().clone();
    assert_eq!(got, vec![1, 2, 3]);
    assert!(buf.is_installed());
}

/// After install, `try_record` returns `Err(ev)`; caller recovers
/// ownership of the event and must dispatch directly.
#[test]
fn record_after_install_returns_err_with_event() {
    let buf = ColdStartBuffer::<u32>::new();
    assert!(buf.try_record(1).is_ok());
    buf.install_callback(|_| {});
    assert_eq!(
        buf.try_record(2),
        Err(2),
        "try_record after install must return Err with the original event",
    );
    assert!(buf.is_installed());
}

/// Exceeding capacity drops further events (returns false), but the
/// already-buffered ones survive.
#[test]
fn capacity_overflow_drops_excess() {
    let buf = ColdStartBuffer::<u32>::new();
    for i in 0..u32::try_from(COLD_START_CAPACITY).expect("fits in u32") {
        assert!(buf.try_record(i).is_ok(), "should accept event #{i}");
    }
    // Next record exceeds capacity — caller recovers the event.
    assert_eq!(
        buf.try_record(99_999_999),
        Err(99_999_999),
        "overflow event must be returned to caller",
    );
    assert_eq!(buf.buffered_count(), Some(COLD_START_CAPACITY));
}

/// Empty buffer drains to zero invocations.
#[test]
fn empty_drain_invokes_callback_zero_times() {
    let buf = ColdStartBuffer::<u32>::new();
    let calls = Arc::new(AtomicUsize::new(0));
    let c = Arc::clone(&calls);
    buf.install_callback(move |_| {
        c.fetch_add(1, Ordering::Relaxed);
    });
    assert_eq!(calls.load(Ordering::Relaxed), 0);
    assert!(buf.is_installed());
}

/// Calling install_callback twice is idempotent — the second call
/// is a no-op (buffer already taken).
#[test]
fn install_callback_twice_is_noop_second_time() {
    let buf = ColdStartBuffer::<u32>::new();
    assert!(buf.try_record(1).is_ok());
    let count1 = Arc::new(AtomicUsize::new(0));
    let c1 = Arc::clone(&count1);
    buf.install_callback(move |_| {
        c1.fetch_add(1, Ordering::Relaxed);
    });
    assert_eq!(count1.load(Ordering::Relaxed), 1);

    let count2 = Arc::new(AtomicUsize::new(0));
    let c2 = Arc::clone(&count2);
    buf.install_callback(move |_| {
        c2.fetch_add(1, Ordering::Relaxed);
    });
    assert_eq!(count2.load(Ordering::Relaxed), 0, "second install is no-op");
}

/// Drain count matches what was recorded (the plan's trace log
/// invariant: `cold_start_drain count=N`).
#[test]
fn drain_count_matches_recorded() {
    let buf = ColdStartBuffer::<&'static str>::new();
    for ev in &["a", "b", "c", "d", "e"] {
        assert!(buf.try_record(*ev).is_ok());
    }
    let count = Arc::new(AtomicUsize::new(0));
    let c = Arc::clone(&count);
    buf.install_callback(move |_| {
        c.fetch_add(1, Ordering::Relaxed);
    });
    assert_eq!(count.load(Ordering::Relaxed), 5);
}
