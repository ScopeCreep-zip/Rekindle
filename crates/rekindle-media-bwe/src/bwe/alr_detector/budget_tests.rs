use super::*;

#[test]
fn test_initial_budget_is_zero() {
    let budget = IntervalBudget::new(Bitrate::kbps(300), true);
    assert_eq!(budget.bytes_remaining(), 0);
    assert_eq!(budget.budget_ratio().to_bits(), 0.0_f64.to_bits());
}

#[test]
fn test_increase_budget_accumulates() {
    let mut budget = IntervalBudget::new(Bitrate::kbps(300), true);

    // Add 100ms worth of budget: 300 kbps * 100ms = 3750 bytes
    budget.increase_budget(Duration::from_millis(100));
    assert_eq!(budget.bytes_remaining(), 3750);

    // Budget ratio should be positive (underuse)
    assert!(budget.budget_ratio() > 0.0);
}

#[test]
fn test_use_budget_creates_debt() {
    let mut budget = IntervalBudget::new(Bitrate::kbps(300), true);

    // Use 5000 bytes
    budget.use_budget(DataSize::bytes(5000));
    assert_eq!(budget.bytes_remaining(), -5000);

    // Budget ratio should be negative (overuse)
    assert!(budget.budget_ratio() < 0.0);
}

#[test]
fn test_budget_ratio_clamped() {
    let mut budget = IntervalBudget::new(Bitrate::kbps(300), true);
    let max_bytes = (Bitrate::kbps(300) * Duration::from_millis(500)).as_bytes_i64();

    // Fill to max
    budget.increase_budget(Duration::from_millis(500));
    assert_eq!(budget.budget_ratio().to_bits(), (1.0_f64).to_bits());

    // Try to add more - should stay at 1.0
    budget.increase_budget(Duration::from_millis(100));
    assert_eq!(budget.budget_ratio().to_bits(), (1.0_f64).to_bits());

    // Use all budget plus create max debt
    budget.use_budget(DataSize::bytes(max_bytes * 2));
    assert_eq!(budget.budget_ratio().to_bits(), (-1.0_f64).to_bits());
}

#[test]
fn test_can_build_up_underuse_false() {
    let mut budget = IntervalBudget::new(Bitrate::kbps(300), false);

    // Add budget
    budget.increase_budget(Duration::from_millis(100));
    let after_first = budget.bytes_remaining();
    assert!(after_first > 0);

    // Add more budget - should reset, not accumulate
    budget.increase_budget(Duration::from_millis(100));
    assert_eq!(budget.bytes_remaining(), 3750); // Only the latest 100ms

    // Not the accumulated 7500
    assert_ne!(budget.bytes_remaining(), after_first + 3750);
}

#[test]
fn test_can_build_up_underuse_true() {
    let mut budget = IntervalBudget::new(Bitrate::kbps(300), true);

    // Add budget
    budget.increase_budget(Duration::from_millis(100));
    let after_first = budget.bytes_remaining();

    // Add more budget - should accumulate
    budget.increase_budget(Duration::from_millis(100));
    assert_eq!(budget.bytes_remaining(), after_first + 3750);
}

#[test]
fn test_debt_recovery() {
    let mut budget = IntervalBudget::new(Bitrate::kbps(300), true);

    // Create debt
    budget.use_budget(DataSize::bytes(5000));
    assert_eq!(budget.bytes_remaining(), -5000);

    // Add budget to recover from debt
    budget.increase_budget(Duration::from_millis(200));
    let expected_recovery = 3750 * 2; // 200ms worth
    assert_eq!(budget.bytes_remaining(), -5000 + expected_recovery);
}

#[test]
fn test_set_target_rate_clamps_budget() {
    let mut budget = IntervalBudget::new(Bitrate::kbps(300), true);

    // Build up budget at 300 kbps
    budget.increase_budget(Duration::from_millis(500));
    let old_max = (Bitrate::kbps(300) * Duration::from_millis(500)).as_bytes_i64();
    assert_eq!(budget.bytes_remaining(), old_max);

    // Reduce target rate - budget should be clamped to new max
    budget.set_target_rate(Bitrate::kbps(150));
    let new_max = (Bitrate::kbps(150) * Duration::from_millis(500)).as_bytes_i64();
    assert_eq!(budget.bytes_remaining(), new_max);
    assert!(new_max < old_max);
}

#[test]
fn test_alr_hysteresis_thresholds() {
    let mut budget = IntervalBudget::new(Bitrate::kbps(300), true);

    // Simulate ALR entry threshold (80% budget ratio)
    let target_bytes = f64_to_i64(
        0.8 * i64_to_f64((Bitrate::kbps(300) * Duration::from_millis(500)).as_bytes_i64()),
    );

    // Accumulate budget to 80% threshold
    while budget.bytes_remaining() < target_bytes {
        budget.increase_budget(Duration::from_millis(50));
    }

    assert!(budget.budget_ratio() >= 0.80);

    // Simulate ALR exit threshold (50% budget ratio)
    // Use budget to bring ratio down
    let use_amount = f64_to_i64(
        0.30 * i64_to_f64((Bitrate::kbps(300) * Duration::from_millis(500)).as_bytes_i64()),
    );
    budget.use_budget(DataSize::bytes(use_amount));

    assert!(budget.budget_ratio() < 0.80);
    assert!(budget.budget_ratio() > 0.40); // Should be around 50%
}

#[test]
fn test_target_rate_getter() {
    let budget = IntervalBudget::new(Bitrate::kbps(500), true);
    assert_eq!(budget.target_rate(), Bitrate::kbps(500));

    // After modifying target rate
    let mut budget = IntervalBudget::new(Bitrate::kbps(300), true);
    budget.set_target_rate(Bitrate::kbps(600));
    assert_eq!(budget.target_rate(), Bitrate::kbps(600));
}
