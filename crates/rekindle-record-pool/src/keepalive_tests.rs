use super::*;

/// The cadence never undercuts Veilid's 300 s rehydration recheck
/// window (0.5.8 `storage_manager/mod.rs:198`), so no warming is a no-op.
#[test]
fn the_cadence_stays_within_its_jitter_and_the_start_within_one_window() {
    for _ in 0..200 {
        let delay = next_warming_delay(&rekindle_utils::OsRandom).as_secs();
        assert!((300..=360).contains(&delay), "delay {delay}");
        assert!(initial_offset(&rekindle_utils::OsRandom) < BASE_INTERVAL);
    }
}
