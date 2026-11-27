use super::*;
use std::time::{Duration, Instant};

#[test]
fn test_window_size_limit() {
    let now = Instant::now();
    let remote_recv_time_base = Instant::now();
    let mut estimator = TrendlineEstimator::new(20);

    estimator.add_delay_observation(
        delay_variation(duration_ms(1), duration_ms(1), remote_recv_time_base),
        now,
    );

    for _ in 0..25 {
        estimator.add_delay_observation(
            delay_variation(
                duration_ms(11),
                duration_ms(1),
                remote_recv_time_base + duration_ms(350),
            ),
            now + duration_ms(500),
        );
    }

    assert_eq!(estimator.history.len(), 20);
}

#[test]
fn test_overuse() {
    let now = Instant::now();
    let remote_recv_time_base = Instant::now();
    let mut estimator = TrendlineEstimator::new(20);

    for g in 0..5 {
        for i in 0..5 {
            estimator.add_delay_observation(
                delay_variation(
                    duration_ms(1),
                    duration_ms(1),
                    remote_recv_time_base + Duration::from_micros(5_000 * g + i * 40),
                ),
                now + duration_ms(g * 100),
            );
        }
    }

    assert_eq!(estimator.hypothesis(), BandwidthUsage::Normal);
    assert_eq!(estimator.history.len(), 20);

    estimator.add_delay_observation(
        delay_variation(
            duration_ms(17),
            duration_ms(5),
            remote_recv_time_base + Duration::from_micros(25_000),
        ),
        now + duration_ms(600),
    );
    assert_eq!(
        estimator.hypothesis(),
        BandwidthUsage::Normal,
        "After getting an initial increasing delay the hypothesis should remain at normal"
    );

    estimator.add_delay_observation(
        delay_variation(
            duration_ms(18),
            duration_ms(5),
            remote_recv_time_base + Duration::from_micros(25_140),
        ),
        now + duration_ms(600),
    );
    assert_eq!(
        estimator.hypothesis(),
        BandwidthUsage::Normal,
        "After getting an a second increasing delay the hypothesis should remain at normal \
            because we the time overusing threshold hasn't been reached yet"
    );

    estimator.add_delay_observation(
        delay_variation(
            duration_ms(22),
            duration_ms(8),
            remote_recv_time_base + Duration::from_micros(25_250),
        ),
        now + duration_ms(600),
    );
    assert_eq!(
        estimator.hypothesis(),
        BandwidthUsage::Overuse,
        "After getting a third increasing delay the hypothesis should move to over because \
            we have been overusing for more than 10ms"
    );
}

fn duration_ms(ms: u64) -> Duration {
    Duration::from_millis(ms)
}

fn delay_variation(
    recv_delta: Duration,
    send_delta: Duration,
    last_remote_recv_time: Instant,
) -> InterGroupDelayDelta {
    InterGroupDelayDelta {
        send_delta: send_delta.into(),
        arrival_delta: recv_delta.into(),
        last_remote_recv_time,
    }
}

#[test]
fn test_history_insertion_regression_out_of_order() {
    // Test for algesten/str0m#698
    //
    // This test reproduces the scenario where a sample with a remote receive time
    // earlier than all existing history entries is added.
    // Prior to the fix, this appended the smallest (negative) element to the end
    // causing the monotonicity assert to fail when the window is full.

    let now = Instant::now();
    let zero_time_base = Instant::now();
    let mut estimator = TrendlineEstimator::new(8);

    // Add a few normal monotonically increasing values
    for offset_ms in [100, 200, 300, 400] {
        estimator.add_delay_observation(
            delay_variation(
                duration_ms(10),
                duration_ms(1),
                zero_time_base + duration_ms(offset_ms),
            ),
            now,
        );
    }

    // Add some out-of-order samples that are earlier than the zero_time
    // (packet reordering is fun).
    for offset_ms in [50, 100, 25, 300] {
        estimator.add_delay_observation(
            delay_variation(
                duration_ms(10),
                duration_ms(1),
                zero_time_base.checked_sub(duration_ms(offset_ms)).unwrap(),
            ),
            now,
        );
    }

    // History should remain sorted.
    let ordered = estimator
        .history
        .iter()
        .zip(estimator.history.iter().skip(1))
        .all(|(a, b)| a.remote_recv_time_ms <= b.remote_recv_time_ms);
    assert!(
        ordered,
        "history should remain sorted by remote_recv_time_ms"
    );
}
