use super::*;

#[test]
fn fresh_until_a_beat_is_missed() {
    let tick = Duration::from_secs(1);
    let heartbeat = Heartbeat::new(tick);
    assert!(heartbeat.is_fresh());
    heartbeat.beat();
    assert!(heartbeat.is_fresh());
    let stale = Heartbeat {
        epoch: Instant::now()
            .checked_sub(tick * 3)
            .expect("monotonic clock covers three ticks"),
        last_ms: AtomicU64::new(0),
        tick,
    };
    assert!(!stale.is_fresh());
    stale.beat();
    assert!(stale.is_fresh());
}
