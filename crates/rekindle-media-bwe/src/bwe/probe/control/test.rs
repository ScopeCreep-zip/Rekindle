//! Unit tests of the probe controller (str0m `src/bwe/probe/control.rs`, `mod test`).

use super::*;

#[test]
fn initial_exponential_probes_are_queued_and_emitted_one_per_tick() {
    let mut pc = ProbeControl::new();
    pc.enable(true);
    let now = Instant::now();

    pc.set_desired_bitrate(Bitrate::mbps(50));
    pc.set_estimated_bitrate(Bitrate::kbps(300), BandwidthLimitedCause::DelayBasedLimited);

    // First handle_timeout triggers initial probing and returns first probe.
    let p1 = pc.handle_timeout(now).unwrap();

    // poll_timeout returns already_happened while there are pending probes.
    assert_eq!(pc.poll_timeout(), already_happened());

    // Second handle_timeout returns the second queued probe.
    let p2 = pc.handle_timeout(now).unwrap();

    assert_eq!(p1.target_bitrate(), Bitrate::kbps(900));
    assert_eq!(p2.target_bitrate(), Bitrate::kbps(1800));
    assert_eq!(p1.min_packet_count(), 5);
    assert_eq!(p1.min_probe_delta(), Duration::from_millis(2));
    assert!(!p1.is_alr_probe());

    // Queue drained - no more probes.
    assert!(pc.handle_timeout(now).is_none());
}

#[test]
fn further_probe_is_triggered_when_probe_result_is_high_enough() {
    let mut pc = ProbeControl::new();
    pc.enable(true);
    let now = Instant::now();

    pc.enable(true);
    pc.set_desired_bitrate(Bitrate::mbps(50));
    pc.set_estimated_bitrate(Bitrate::mbps(1), BandwidthLimitedCause::DelayBasedLimited);

    // Drain initial two probes.
    let _ = pc.handle_timeout(now).unwrap();
    let _ = pc.handle_timeout(now).unwrap();

    // WebRTC rule: if measured bitrate > min_bitrate_to_probe_further, probe at 2x measured.
    // min_bitrate_to_probe_further is 0.7 * last_probe_rate (6x start) = 4.2 Mbps.
    pc.set_estimated_bitrate(Bitrate::mbps(5), BandwidthLimitedCause::DelayBasedLimited);

    let p = pc.handle_timeout(now + Duration::from_millis(10)).unwrap();
    assert_eq!(p.target_bitrate(), Bitrate::mbps(10));
}

#[test]
fn allocation_probe_is_triggered_in_alr_when_allocation_increases() {
    let mut pc = ProbeControl::new();
    pc.enable(true);
    let now = Instant::now();

    pc.set_desired_bitrate(Bitrate::mbps(1));
    pc.set_estimated_bitrate(Bitrate::mbps(1), BandwidthLimitedCause::DelayBasedLimited);

    // Drain initial probes.
    let _ = pc.handle_timeout(now).unwrap();
    let _ = pc.handle_timeout(now).unwrap();

    // Time out waiting for probing result -> probing complete.
    assert!(pc.handle_timeout(now + Duration::from_secs(2)).is_none());

    // Enter ALR
    pc.set_alr_start_time(now + Duration::from_secs(2));

    // No probe yet - desired hasn't increased
    assert!(pc.handle_timeout(now + Duration::from_secs(2)).is_none());

    // Increase desired bitrate while in ALR (desired > prev AND desired > estimate)
    pc.set_desired_bitrate(Bitrate::mbps(4));

    // Should trigger allocation probe: p1 = 4 Mbps * 1.0 = 4 Mbps, capped by 2× estimate = 2 Mbps
    let p = pc.handle_timeout(now + Duration::from_secs(2)).unwrap();
    assert_eq!(p.target_bitrate(), Bitrate::mbps(2));
}

#[test]
fn handles_bitrate_infinity_without_panic() {
    let mut pc = ProbeControl::new();
    pc.enable(true);
    let now = Instant::now();

    pc.set_desired_bitrate(Bitrate::mbps(50));

    // Should not panic with Infinity
    pc.set_estimated_bitrate(Bitrate::INFINITY, BandwidthLimitedCause::DelayBasedLimited);

    // Verify behavior is reasonable (no probing with infinite estimate)
    assert!(pc.handle_timeout(now).is_none());
}

#[test]
fn handles_clock_skew_gracefully() {
    let mut pc = ProbeControl::new();
    pc.enable(true);
    let now = Instant::now();

    pc.set_desired_bitrate(Bitrate::mbps(50));
    pc.set_estimated_bitrate(Bitrate::kbps(300), BandwidthLimitedCause::DelayBasedLimited);

    // Drain initial probes
    let _ = pc.handle_timeout(now);
    let _ = pc.handle_timeout(now);

    // Simulate time going backwards (clock skew)
    let earlier = now.checked_sub(Duration::from_secs(5)).unwrap();

    // Should handle gracefully with saturating_duration_since
    let _ = pc.handle_timeout(earlier);

    // Should still be able to continue normally
    let _ = pc.handle_timeout(now + Duration::from_secs(1));
}

#[test]
fn handles_max_bitrate_zero() {
    let mut pc = ProbeControl::new();
    pc.enable(true);
    let now = Instant::now();

    // Set max_bitrate to zero - this is rejected as first value to avoid
    // creating probes with zero cap.
    pc.set_desired_bitrate(Bitrate::ZERO);
    pc.set_estimated_bitrate(Bitrate::kbps(300), BandwidthLimitedCause::DelayBasedLimited);

    // No probes should be created since desired was rejected.
    let p1 = pc.handle_timeout(now);
    assert!(p1.is_none(), "Should not create probes with zero desired");
}

#[test]
fn allocation_probe_fires_when_desired_increases_in_alr() {
    let mut pc = ProbeControl::new();
    pc.enable(true);
    let now = Instant::now();

    pc.set_desired_bitrate(Bitrate::kbps(500));
    pc.set_estimated_bitrate(Bitrate::kbps(500), BandwidthLimitedCause::DelayBasedLimited);

    // Drain initial probes
    let _ = pc.handle_timeout(now);
    let _ = pc.handle_timeout(now);

    // Timeout to reach probing complete
    assert!(pc.handle_timeout(now + Duration::from_secs(2)).is_none());

    // Enter ALR
    pc.set_alr_start_time(now + Duration::from_secs(3));

    // No probe on ALR entry alone
    assert!(pc.handle_timeout(now + Duration::from_secs(3)).is_none());

    // Increase desired while in ALR (desired > prev AND desired > estimate)
    pc.set_desired_bitrate(Bitrate::mbps(4));

    // Should trigger allocation probe
    let probe = pc.handle_timeout(now + Duration::from_secs(3));
    assert!(
        probe.is_some(),
        "Allocation probe should trigger when desired increases in ALR"
    );
}

#[test]
fn large_drop_probing_after_alr_ended() {
    let mut pc = ProbeControl::new();
    pc.enable(true);
    let now = Instant::now();

    pc.set_desired_bitrate(Bitrate::mbps(5));
    pc.set_estimated_bitrate(Bitrate::mbps(5), BandwidthLimitedCause::DelayBasedLimited);

    // Drain initial probes
    let _ = pc.handle_timeout(now);
    let _ = pc.handle_timeout(now);

    // Timeout to probing complete
    assert!(pc.handle_timeout(now + Duration::from_secs(2)).is_none());

    // Enter and exit ALR (large-drop works when ALR ended recently)
    pc.set_alr_start_time(now + Duration::from_secs(2));
    pc.set_alr_stop_time(now + Duration::from_secs(3));

    // Simulate large drop (to 60% of original = 3 Mbps, below 66% threshold)
    pc.set_estimated_bitrate(Bitrate::mbps(3), BandwidthLimitedCause::DelayBasedLimited);

    // Check at now+5s (within 3s of ALR ending, so alr_ended_recently is true)
    let later = now + Duration::from_secs(5);

    // Should trigger large-drop recovery probe at 85% of pre-drop rate (4.25 Mbps)
    let p = pc.handle_timeout(later);
    assert!(p.is_some(), "Large-drop recovery should schedule probe");
    if let Some(probe) = p {
        // 85% of 5 Mbps = 4.25 Mbps
        assert!(probe.target_bitrate() >= Bitrate::mbps(4));
        assert!(probe.target_bitrate() <= Bitrate::mbps(5));
    }
}

#[test]
fn allocation_probe_requires_desired_increase_in_alr() {
    let mut pc = ProbeControl::new();
    pc.enable(true);
    let now = Instant::now();

    pc.set_desired_bitrate(Bitrate::mbps(5));
    pc.set_estimated_bitrate(Bitrate::mbps(1), BandwidthLimitedCause::DelayBasedLimited);

    // Drain initial probes
    let _ = pc.handle_timeout(now);
    let _ = pc.handle_timeout(now);

    // Timeout to probing complete
    assert!(pc.handle_timeout(now + Duration::from_secs(2)).is_none());

    // Enter ALR with estimate < max_bitrate
    pc.set_alr_start_time(now + Duration::from_secs(2));

    // No allocation probe on ALR entry - need desired to increase
    let probe = pc.handle_timeout(now + Duration::from_secs(2));
    assert!(
        probe.is_none(),
        "Should NOT trigger allocation probe on ALR entry alone"
    );

    // Increase desired while in ALR
    pc.set_desired_bitrate(Bitrate::mbps(10));

    // Now should trigger allocation probe
    let probe = pc.handle_timeout(now + Duration::from_secs(2));
    assert!(
        probe.is_some(),
        "Should trigger allocation probe when desired increases in ALR"
    );
}

#[test]
fn no_periodic_alr_probing_by_default() {
    let mut pc = ProbeControl::new();
    assert!(!pc.config.periodic_alr_probing);
    pc.enable(true);
    let now = Instant::now();
    pc.set_alr_start_time(now);
    let later = now + Duration::from_secs(30);
    assert!(!pc.maybe_periodic_alr(later, Bitrate::mbps(2)));
}

#[test]
fn periodic_alr_probing() {
    let mut pc = ProbeControl::new();
    pc.config.periodic_alr_probing = true;
    pc.enable(true);
    let now = Instant::now();

    pc.set_desired_bitrate(Bitrate::mbps(5));
    pc.set_estimated_bitrate(Bitrate::mbps(1), BandwidthLimitedCause::DelayBasedLimited);

    // Drain initial probes
    let _ = pc.handle_timeout(now);
    let _ = pc.handle_timeout(now);

    // Timeout to probing complete
    assert!(pc.handle_timeout(now + Duration::from_secs(2)).is_none());

    // Enter ALR
    pc.set_alr_start_time(now + Duration::from_secs(2));

    // No immediate probe on ALR entry
    assert!(pc.handle_timeout(now + Duration::from_secs(2)).is_none());

    // Wait 5 seconds for periodic probe (2s to complete initial + 5s = 7s)
    let probe = pc.handle_timeout(now + Duration::from_secs(7));
    assert!(
        probe.is_some(),
        "Should trigger periodic ALR probe after 5 seconds in ALR"
    );
    assert!(probe.unwrap().is_alr_probe());
}

#[test]
fn periodic_alr_probing_continues_even_when_estimate_reaches_max() {
    let mut pc = ProbeControl::new();
    pc.config.periodic_alr_probing = true;
    pc.enable(true);
    let now = Instant::now();

    pc.set_desired_bitrate(Bitrate::mbps(2));
    pc.set_estimated_bitrate(Bitrate::mbps(1), BandwidthLimitedCause::DelayBasedLimited);

    // Drain initial probes
    let _ = pc.handle_timeout(now);
    let _ = pc.handle_timeout(now);

    // Timeout to probing complete
    assert!(pc.handle_timeout(now + Duration::from_secs(2)).is_none());

    // Enter ALR
    pc.set_alr_start_time(now + Duration::from_secs(2));

    // No immediate probe on ALR entry
    assert!(pc.handle_timeout(now + Duration::from_secs(2)).is_none());

    // Now increase estimate to match max_bitrate
    pc.set_estimated_bitrate(Bitrate::mbps(2), BandwidthLimitedCause::DelayBasedLimited);

    // Wait 5 seconds - should still trigger periodic probe in ALR
    // even though estimate >= max_bitrate, to maintain confidence in the estimate
    let probe = pc.handle_timeout(now + Duration::from_secs(7));
    assert!(
        probe.is_some(),
        "Should continue periodic probing in ALR even when estimate >= max_bitrate"
    );
}
