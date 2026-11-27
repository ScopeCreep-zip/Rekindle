use super::*;

#[test]
fn a_seed_and_index_fix_the_stream() {
    let take = |seed, index| {
        let rng = SimRandom::new(seed, index);
        (rng.next_u32(), rng.next_u32())
    };
    assert_eq!(take(7, 0), take(7, 0));
    assert_ne!(take(7, 0), take(7, 1));
    assert_ne!(take(7, 0), take(8, 0));
}

#[tokio::test(start_paused = true)]
async fn the_clock_follows_the_paused_runtime_and_its_skew() {
    let clock = SimClock::new(tokio::time::Instant::now());
    assert_eq!(clock.now_ms(), SIM_EPOCH_MS);
    tokio::time::advance(std::time::Duration::from_secs(90)).await;
    assert_eq!(clock.now_secs(), SIM_EPOCH_MS / 1000 + 90);
    clock.set_skew_ms(-3_600_000);
    assert_eq!(clock.now_ms(), SIM_EPOCH_MS + 90_000 - 3_600_000);
}

#[tokio::test(start_paused = true)]
async fn a_sleeping_clock_stands_still_then_jumps_across_the_sleep() {
    let clock = SimClock::new(tokio::time::Instant::now());
    clock.freeze(true);
    tokio::time::advance(std::time::Duration::from_secs(600)).await;
    assert_eq!(clock.now_ms(), SIM_EPOCH_MS, "asleep, no time passes");
    clock.freeze(false);
    assert_eq!(clock.now_ms(), SIM_EPOCH_MS + 600_000, "the wake's jump");
}
