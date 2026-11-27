use super::*;

#[test]
fn starts_with_no_estimate() {
    let estimator = LinkCapacityEstimator::new();
    let now = Instant::now();

    assert_eq!(estimator.capacity_estimate(now), None);
    assert!(!estimator.has_estimate());
}

#[test]
fn stores_probe_result() {
    let mut estimator = LinkCapacityEstimator::new();
    let now = Instant::now();
    let probe_result = Bitrate::mbps(10);

    estimator.update_from_probe(probe_result, now);

    assert_eq!(estimator.capacity_estimate(now), Some(probe_result));
    assert!(estimator.has_estimate());
}

#[test]
fn takes_maximum_of_multiple_probes() {
    let mut estimator = LinkCapacityEstimator::new();
    let now = Instant::now();

    estimator.update_from_probe(Bitrate::mbps(10), now);
    estimator.update_from_probe(Bitrate::mbps(5), now);

    // Should keep the higher estimate
    assert_eq!(estimator.capacity_estimate(now), Some(Bitrate::mbps(10)));

    estimator.update_from_probe(Bitrate::mbps(15), now);

    // Should update to higher estimate
    assert_eq!(estimator.capacity_estimate(now), Some(Bitrate::mbps(15)));
}

#[test]
fn estimate_expires_after_reset_window() {
    let mut estimator = LinkCapacityEstimator::new();
    let now = Instant::now();

    estimator.update_from_probe(Bitrate::mbps(10), now);
    assert_eq!(estimator.capacity_estimate(now), Some(Bitrate::mbps(10)));

    // Check just before expiration
    let almost_expired = now + Duration::from_secs(59);
    assert_eq!(
        estimator.capacity_estimate(almost_expired),
        Some(Bitrate::mbps(10))
    );

    // Check after expiration
    let expired = now + Duration::from_secs(61);
    assert_eq!(estimator.capacity_estimate(expired), None);
}

#[test]
fn reset_clears_estimate() {
    let mut estimator = LinkCapacityEstimator::new();
    let now = Instant::now();

    estimator.update_from_probe(Bitrate::mbps(10), now);
    assert!(estimator.has_estimate());

    estimator.reset();

    assert!(!estimator.has_estimate());
    assert_eq!(estimator.capacity_estimate(now), None);
}

#[test]
fn ignores_invalid_probes() {
    let mut estimator = LinkCapacityEstimator::new();
    let now = Instant::now();

    // Set a valid estimate first
    estimator.update_from_probe(Bitrate::mbps(10), now);

    // Try to update with invalid bitrate
    estimator.update_from_probe(Bitrate::NEG_INFINITY, now);

    // Should keep the valid estimate
    assert_eq!(estimator.capacity_estimate(now), Some(Bitrate::mbps(10)));
}
