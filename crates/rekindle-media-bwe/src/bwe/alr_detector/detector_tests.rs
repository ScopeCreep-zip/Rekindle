use super::*;

#[test]
fn timeout_detects_complete_silence() {
    let now = Instant::now();
    let mut detector = AlrDetector::new();
    detector.set_estimated_bitrate(Bitrate::mbps(1));
    detector.handle_timeout(now);
    detector.handle_timeout(now + Duration::from_millis(500));
    assert!(detector.alr_start_time().is_some());
}

#[test]
fn frequent_timeouts_preserve_media_budget() {
    let now = Instant::now();
    let mut with_timeouts = AlrDetector::new();
    let mut packet_driven = AlrDetector::new();
    for detector in [&mut with_timeouts, &mut packet_driven] {
        detector.set_estimated_bitrate(Bitrate::kbps(250));
        detector.on_bytes_sent(DataSize::ZERO, now);
    }
    for tick in 1..10_000 {
        let now = now + Duration::from_micros(tick * 100);
        with_timeouts.handle_timeout(now);
        if tick % 50 == 0 {
            for detector in [&mut with_timeouts, &mut packet_driven] {
                detector.on_bytes_sent(DataSize::bytes(100), now);
            }
            assert_eq!(
                with_timeouts.budget_ratio().to_bits(),
                packet_driven.budget_ratio().to_bits()
            );
            assert_eq!(
                with_timeouts.alr_start_time(),
                packet_driven.alr_start_time()
            );
        }
    }
}

#[test]
fn test_alr_not_detected_initially() {
    let alr = AlrDetector::new();
    assert!(alr.alr_start_time().is_none());
}

#[test]
fn test_alr_enter_with_low_usage() {
    let mut alr = AlrDetector::new();
    let now = Instant::now();

    // Set estimate to 1 Mbps (target will be 650 kbps)
    alr.set_estimated_bitrate(Bitrate::mbps(1));

    // Send very little data (way below target)
    // Over 500ms, we should accumulate budget
    for i in 0..50 {
        let t = now + Duration::from_millis(i * 10);
        // Send 100 bytes every 10ms = 80 kbps (way below 650 kbps target)
        alr.on_bytes_sent(DataSize::bytes(100), t);
    }

    // Should enter ALR
    assert!(alr.alr_start_time().is_some());
    assert!(alr.budget_ratio() > 0.80);
}

#[test]
fn test_alr_exit_with_high_usage() {
    let mut alr = AlrDetector::new();
    let now = Instant::now();

    alr.set_estimated_bitrate(Bitrate::mbps(1));

    // Enter ALR with low usage
    for i in 0..50 {
        let t = now + Duration::from_millis(i * 10);
        alr.on_bytes_sent(DataSize::bytes(100), t);
    }
    assert!(alr.alr_start_time().is_some());

    // Now send at high rate to exit ALR
    let start_exit = now + Duration::from_millis(500);
    for i in 0..50 {
        let t = start_exit + Duration::from_millis(i * 10);
        // Send 10,000 bytes every 10ms = 8 Mbps (way above 650 kbps target)
        alr.on_bytes_sent(DataSize::bytes(10_000), t);
    }

    // Should exit ALR
    assert!(alr.alr_start_time().is_none());
    assert!(alr.budget_ratio() < 0.50);
}

#[test]
fn test_alr_hysteresis() {
    let mut alr = AlrDetector::new();
    let now = Instant::now();

    alr.set_estimated_bitrate(Bitrate::mbps(1));

    // Send at exactly 65% (the target) - should not enter ALR
    for i in 0..100 {
        let t = now + Duration::from_millis(i * 10);
        // 65% of 1 Mbps = 650 kbps = 812.5 bytes per 10ms
        alr.on_bytes_sent(DataSize::bytes(812), t);
    }

    // Budget ratio should be near 0, not trigger ALR entry (needs > 0.80)
    assert!(alr.alr_start_time().is_none());
    let ratio = alr.budget_ratio();
    assert!(ratio < 0.80, "ratio={ratio}");
    assert!(ratio > -0.20, "ratio={ratio}"); // Some tolerance
}

#[test]
fn test_alr_handles_bursts() {
    let mut alr = AlrDetector::new();
    let now = Instant::now();

    alr.set_estimated_bitrate(Bitrate::mbps(1));

    // Simulate bursty encoder: alternate between large and small frames
    let mut time = now;
    for i in 0..100 {
        time += Duration::from_millis(33); // ~30fps

        if i % 10 == 0 {
            // Keyframe: 10x normal size
            alr.on_bytes_sent(DataSize::bytes(50_000), time);
        } else {
            // Normal frame: small
            alr.on_bytes_sent(DataSize::bytes(5_000), time);
        }
    }

    // Average is about 9.5 KB per frame = 285 KB/s = 2.28 Mbps
    // This is above the 650 kbps target, so should NOT be in ALR
    // The IntervalBudget's 500ms window should smooth out the bursts
    assert!(alr.alr_start_time().is_none());
}

#[test]
fn test_alr_estimate_change() {
    let mut alr = AlrDetector::new();
    let now = Instant::now();

    // Start with 1 Mbps estimate (target = 650 kbps)
    alr.set_estimated_bitrate(Bitrate::mbps(1));

    // Send at 200 kbps (way below 650 kbps target) to accumulate budget quickly
    for i in 0..100 {
        let t = now + Duration::from_millis(i * 10);
        alr.on_bytes_sent(DataSize::bytes(250), t); // 200 kbps
    }

    // Should enter ALR after sending well below target
    assert!(alr.alr_start_time().is_some());

    // Now BWE drops to 300 kbps (target becomes 195 kbps)
    alr.set_estimated_bitrate(Bitrate::kbps(300));

    // Continue sending at 250 kbps - now above target (195 kbps)
    let continue_time = now + Duration::from_millis(1000);
    for i in 0..100 {
        let t = continue_time + Duration::from_millis(i * 10);
        alr.on_bytes_sent(DataSize::bytes(312), t); // 250 kbps
    }

    // Should exit ALR (now sending above target)
    assert!(alr.alr_start_time().is_none());
}

#[test]
fn test_alr_should_not_trigger_when_sending_at_target_rate() {
    // This test verifies that ALR doesn't incorrectly trigger when sending
    // at the target rate, even with small time advances between packets.
    let mut alr = AlrDetector::new();
    let now = Instant::now();

    // Set estimate to 8.25 Mbps (target will be 5.36 Mbps at 65%)
    let estimate = Bitrate::kbps(8250);
    alr.set_estimated_bitrate(estimate);

    // Send at exactly 8.25 Mbps with realistic packet timing
    // Packet: 1150 bytes
    // Interval: 1150 bytes × 8 bits / 8,250,000 bps = 1.115ms
    let packet_size = DataSize::bytes(1150);
    let packet_interval = Duration::from_micros(1115); // 1.115ms

    // Send 100 packets over ~111ms
    for i in 0..100 {
        let t = now + packet_interval * i;
        alr.on_bytes_sent(packet_size, t);
    }

    // Should NOT be in ALR - we're sending at 8.25 Mbps, which is above
    // the target threshold of 5.36 Mbps (65% of estimate)
    assert!(
        alr.alr_start_time().is_none(),
        "ALR should not trigger when sending at target rate. Budget ratio: {:.3}",
        alr.budget_ratio()
    );
}

#[test]
fn test_alr_handles_spurious_small_time_advances() {
    // This test verifies that ALR works correctly even when time advances
    // in very small increments (like the test harness's forced 0.2ms advances).
    let mut alr = AlrDetector::new();
    let now = Instant::now();

    // Set estimate to 8.25 Mbps
    let estimate = Bitrate::kbps(8250);
    alr.set_estimated_bitrate(estimate);

    // Simulate sending 1150 byte packets at 8.25 Mbps
    // But with spurious 0.2ms time advances between on_bytes_sent calls
    let packet_size = DataSize::bytes(1150);
    let spurious_advance = Duration::from_micros(200); // 0.2ms

    let mut time = now;
    // Send packets - but accumulate proper time even if called frequently
    for _ in 0..100 {
        alr.on_bytes_sent(packet_size, time);
        time += spurious_advance; // Small spurious advance
    }

    // With 100 packets over 20ms (100 × 0.2ms), we've sent:
    // 115,000 bytes in 20ms = 46 Mbps
    // This is WAY above target, so ALR should NOT trigger
    assert!(
        alr.alr_start_time().is_none(),
        "ALR should not trigger with spurious small advances when sending fast. Budget ratio: {:.3}",
        alr.budget_ratio()
    );
}
