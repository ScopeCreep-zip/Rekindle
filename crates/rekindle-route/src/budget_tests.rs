use super::*;

const HOUR_MS: u64 = 60 * 60 * 1000;

/// The budget refuses past its capacity, and nothing refills it
/// within the day.
#[test]
fn the_budget_refuses_past_its_capacity() {
    let mut budget = RouteBudget::full(0);
    for _ in 0..BUDGET_CAPACITY {
        assert!(budget.try_spend(HOUR_MS));
    }
    assert!(!budget.try_spend(HOUR_MS));
    assert!(!budget.try_spend(23 * HOUR_MS));
    assert!(budget.try_spend(24 * HOUR_MS), "a day refills it");
}

/// A seeded bucket starts with its capacity and a day's refill stops
/// there, never at the default capacity; a seed of 0 refuses the first
/// spend.
#[test]
fn a_seeded_bucket_holds_its_capacity() {
    let mut budget = RouteBudget::seeded(2, 0);
    assert!(budget.try_spend(HOUR_MS));
    assert!(budget.try_spend(HOUR_MS));
    assert!(!budget.try_spend(HOUR_MS));
    assert!(budget.try_spend(48 * HOUR_MS), "a day refills it");
    assert!(budget.try_spend(48 * HOUR_MS));
    assert!(!budget.try_spend(48 * HOUR_MS), "but only to its capacity");
    assert!(!RouteBudget::seeded(0, 0).try_spend(0));
}

/// A storm that spends every day's refill stops at the weekly cap.
#[test]
fn a_storm_stops_at_the_weekly_cap() {
    let mut budget = RouteBudget::full(0);
    let mut spent = 0;
    for hour in 0..(6 * 24) {
        while budget.try_spend(hour * HOUR_MS) {
            spent += 1;
        }
    }
    assert_eq!(spent, BUDGET_WEEKLY_CAP);
    assert!(budget.try_spend(7 * 24 * HOUR_MS), "a new week");
}

/// Deaths above the baseline within the window warn once; the natural
/// rate never does.
#[test]
fn deaths_above_the_baseline_warn_once() {
    let start = tokio::time::Instant::now();
    let d = |total, young| OwnDeaths { total, young };
    let mut rate = DeathRate::new(d(2, 1));
    for n in 1..=DEATH_WARN_ABOVE {
        assert_eq!(rate.observe(d(2 + n, 1), start), None, "at the baseline");
    }
    assert_eq!(
        rate.observe(d(3 + DEATH_WARN_ABOVE, 3), start),
        Some(d(DEATH_WARN_ABOVE + 1, 2)),
        "young deaths within the window are told apart"
    );
    assert_eq!(
        rate.observe(d(4 + DEATH_WARN_ABOVE, 3), start),
        None,
        "warned once"
    );
    let later = start + DEATH_WINDOW;
    assert_eq!(
        rate.observe(d(4 + DEATH_WARN_ABOVE, 3), later),
        None,
        "the window passed"
    );
    let mut natural = DeathRate::new(d(0, 0));
    for hour in 0..48u32 {
        let at = start + Duration::from_secs(u64::from(hour) * 3600);
        assert_eq!(natural.observe(d(hour / 5, hour / 10), at), None);
    }
}

/// A clock set back refills nothing.
#[test]
fn a_clock_set_back_refills_nothing() {
    let mut budget = RouteBudget::full(10 * 24 * HOUR_MS);
    budget.state.tokens = 0;
    assert!(!budget.try_spend(0));
}
