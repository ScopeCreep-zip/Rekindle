use super::*;

#[test]
fn test_no_estimate_before_initial_window_has_passed() {
    let now = Instant::now();
    let mut estimator =
        AckedBitrateEstimator::new(Duration::from_millis(500), Duration::from_millis(150));

    estimator.update(now, DataSize::bytes(950));
    estimator.update(now + Duration::from_millis(250), DataSize::bytes(381));
    estimator.update(now + Duration::from_millis(499), DataSize::bytes(1110));

    assert!(
        estimator.current_estimate().is_none(),
        "AckedBitrateEstiamtor should produce no estimate before the initial window is reached"
    );

    estimator.update(now + Duration::from_millis(501), DataSize::bytes(1110));

    let estimate = estimator.current_estimate().expect(
        "After the first window of time has passed AckedBitrateEstimator should produce an estimate"
    );

    assert_eq!(
        estimate.as_u64(),
        39056,
        "AckedBitrateEstiamtor should produce the correct bitrate"
    );
}

#[test]
fn test_correct_estimate_after_initial_window() {
    let now = Instant::now();
    let mut estimator =
        AckedBitrateEstimator::new(Duration::from_millis(500), Duration::from_millis(150));

    estimator.update(now, DataSize::bytes(2500));
    estimator.update(now + Duration::from_millis(250), DataSize::bytes(1392));
    estimator.update(now + Duration::from_millis(499), DataSize::bytes(4021));
    estimator.update(now + Duration::from_millis(500), DataSize::bytes(0));

    assert!(
        estimator.current_estimate().is_some(),
        "After the first window of time has passed AckedBitrateEstimator should produce an estimate"
    );

    estimator.update(now + Duration::from_millis(550), DataSize::bytes(271));
    estimator.update(now + Duration::from_millis(558), DataSize::bytes(813));
    estimator.update(now + Duration::from_millis(648), DataSize::bytes(731));
    // Will not be counted, part of next window
    estimator.update(now + Duration::from_millis(651), DataSize::bytes(900));

    let estimate = estimator.current_estimate().expect(
        "After the first window of time has passed AckedBitrateEstimator should produce an estimate"
    );

    assert_eq!(
        estimate.as_u64(),
        108_320,
        "AckedBitrateEstiamtor should produce the correct bitrate"
    );
}

#[test]
fn test_long_gap_preserves_overflow_via_modulo_after_initial() {
    let now = Instant::now();
    let window = Duration::from_millis(150);
    let mut estimator = AckedBitrateEstimator::new(Duration::from_millis(500), window);

    // First, get past the initial 500ms window to establish an estimate
    // Need to send enough data over 500ms to trigger the first estimate
    for i in 0..50 {
        estimator.update(now + Duration::from_millis(i * 10), DataSize::bytes(1000));
    }
    estimator.update(now + Duration::from_millis(501), DataSize::bytes(1000));
    assert!(
        estimator.current_estimate().is_some(),
        "Should have initial estimate after 500ms"
    );

    // Now we're using the 150ms window
    // Build up current_window to 100ms
    estimator.update(now + Duration::from_millis(550), DataSize::bytes(1000));
    estimator.update(now + Duration::from_millis(650), DataSize::bytes(1000));
    // current_window is now 100ms

    // Long gap of 225ms (> 150ms window)
    // current_window becomes 100 + 225 = 325ms
    // After gap detection: current_window = 325ms % 150ms = 25ms (correct)
    //                  or: current_window = 150ms % 150ms = 0ms (buggy)
    estimator.update(now + Duration::from_millis(875), DataSize::bytes(1000));

    // Add 50ms
    estimator.update(now + Duration::from_millis(925), DataSize::bytes(1000));
    // Correct: current_window = 25 + 50 = 75ms
    // Buggy: current_window = 0 + 50 = 50ms

    // Add 80ms
    let estimate_before = estimator.current_estimate().unwrap();
    estimator.update(now + Duration::from_millis(1005), DataSize::bytes(1000));
    // Correct: current_window = 75 + 80 = 155ms (>= 150ms, triggers new sample)
    // Buggy: current_window = 50 + 80 = 130ms (< 150ms, no new sample)
    let estimate_after = estimator.current_estimate().unwrap();

    // With correct modulo, estimate should change (new sample produced)
    // With buggy modulo, estimate stays the same (no sample produced yet)
    assert_ne!(
        estimate_before.as_u64(),
        estimate_after.as_u64(),
        "Estimate should change after accumulating >= 150ms due to correct modulo overflow"
    );
}
