//! Unit tests of the probe estimator (str0m `src/bwe/probe/estimator.rs`, `mod test`).

use super::*;
use crate::bwe::probe::ProbeKind;
use crate::{TwccPacketId, TwccSeq};

#[test]
fn probe_estimator_starts_with_no_active_probe() {
    let estimator = ProbeEstimator::new();
    assert_eq!(estimator.poll_timeout(), not_happening());
}

#[test]
fn probe_estimator_lifecycle() {
    let mut estimator = ProbeEstimator::new();
    let now = Instant::now();

    // Start probe
    let config = ProbeClusterConfig::new(1.into(), Bitrate::mbps(2), ProbeKind::Initial);
    assert!(estimator.probe_start(config, now));
    assert!(estimator.states.len() == 1, "Should have one active probe");
    assert_eq!(estimator.poll_timeout(), not_happening());

    // End probe with 1 second cluster history retention
    estimator.end_probe(now, config.cluster());
    let timeout = estimator.poll_timeout();
    assert!(
        timeout > now && timeout <= now + Duration::from_secs(1),
        "Expected timeout between now and now+1s, got: {:?}",
        timeout.duration_since(now)
    );

    // Handle timeout clears expired probes
    estimator.handle_timeout(now + Duration::from_secs(1));
    assert!(estimator.states.is_empty(), "All probes should be cleared");
    assert_eq!(estimator.poll_timeout(), not_happening());
}

#[test]
fn lost_probe_packets_do_not_affect_estimate() {
    let mut estimator = ProbeEstimator::new();
    let cluster: TwccClusterId = 7.into();
    let config = ProbeClusterConfig::new(cluster, Bitrate::mbps(2), ProbeKind::Initial);

    let base = Instant::now();
    let received = (0..5).map(|i| {
        let seq: TwccSeq = (1000 + i).into();
        let pid = TwccPacketId::with_cluster(seq, cluster);
        // send spaced 4ms, recv spaced 4ms (same ordering)
        crate::TwccSendRecord::test_new(
            pid,
            base + Duration::from_millis(i * 4),
            1200,
            base + Duration::from_millis(i * 4 + 1),
            Some(base + Duration::from_millis(i * 4 + 2)),
        )
    });

    // Add extra lost probe packets with later send times. These should not change the result.
    let lost = (0..20).map(|i| {
        let seq: TwccSeq = (2000 + i).into();
        let pid = TwccPacketId::with_cluster(seq, cluster);
        crate::TwccSendRecord::test_new(
            pid,
            base + Duration::from_millis(100 + i),
            1200,
            base + Duration::from_millis(150 + i),
            None, // lost
        )
    });

    // First run: only received packets
    estimator.probe_start(config, base);
    let recv_vec: Vec<_> = received.collect();
    let results: Vec<_> = estimator.update(recv_vec.iter()).collect();
    let estimate_only_received = results
        .last()
        .map(|(_, bitrate)| *bitrate)
        .expect("expected a probe estimate");

    // Second run: received + lost
    let mut estimator2 = ProbeEstimator::new();
    estimator2.probe_start(config, base);
    let mut all_vec = recv_vec;
    all_vec.extend(lost);
    let results: Vec<_> = estimator2.update(all_vec.iter()).collect();
    let estimate_with_lost = results
        .last()
        .map(|(_, bitrate)| *bitrate)
        .expect("expected a probe estimate");

    assert_eq!(
        estimate_only_received, estimate_with_lost,
        "lost packets must not change probe estimate"
    );
}

#[test]
fn invalid_receive_send_ratio_is_rejected() {
    let mut estimator = ProbeEstimator::new();
    let cluster: TwccClusterId = 9.into();
    let config = ProbeClusterConfig::new(cluster, Bitrate::mbps(2), ProbeKind::Initial);

    let base = Instant::now();
    // Make send times span 200ms, but receive times span only 1ms.
    // This yields receive_rate >> send_rate. WebRTC would reject this via ratio check.
    let records: Vec<_> = (0..5)
        .map(|i| {
            let seq: TwccSeq = (3000 + i).into();
            let pid = TwccPacketId::with_cluster(seq, cluster);
            crate::TwccSendRecord::test_new(
                pid,
                base + Duration::from_millis(i * 50),
                1200,
                base + Duration::from_millis(250 + i),
                Some(base + Duration::from_millis(300 + (i % 2))), // ~0-1ms spread
            )
        })
        .collect();

    estimator.probe_start(config, base);
    let results: Vec<_> = estimator.update(records.iter()).collect();

    assert!(
        results.is_empty(),
        "probe should be rejected by ratio validation, got: {results:?}"
    );
}

#[test]
fn send_interval_zero_is_rejected() {
    let mut estimator = ProbeEstimator::new();
    let cluster: TwccClusterId = 10.into();
    let config = ProbeClusterConfig::new(cluster, Bitrate::mbps(2), ProbeKind::Initial);

    let base = Instant::now();
    // All packets have the same send time -> send_interval == 0.
    let records: Vec<_> = (0..5)
        .map(|i| {
            let seq: TwccSeq = (4000 + i).into();
            let pid = TwccPacketId::with_cluster(seq, cluster);
            crate::TwccSendRecord::test_new(
                pid,
                base, // identical send time for all
                1200,
                base + Duration::from_millis(10 + i),
                Some(base + Duration::from_millis(20 + i)),
            )
        })
        .collect();

    estimator.probe_start(config, base);
    let results: Vec<_> = estimator.update(records.iter()).collect();

    assert!(
        results.is_empty(),
        "send_interval == 0 should be rejected, got: {results:?}"
    );
}

#[test]
fn stale_probes_cleaned_up_at_capacity() {
    let mut estimator = ProbeEstimator::new();
    let base = Instant::now();

    // Fill up to MAX_ACTIVE_PROBES with probes created at `base`
    for i in 0..MAX_ACTIVE_PROBES {
        let config =
            ProbeClusterConfig::new((i as u64).into(), Bitrate::mbps(2), ProbeKind::Initial);
        assert!(
            estimator.probe_start(config, base),
            "should accept probe {i}"
        );
    }
    assert_eq!(estimator.states.len(), MAX_ACTIVE_PROBES);

    // Try to add another probe at the same time - should be rejected
    let config = ProbeClusterConfig::new(100.into(), Bitrate::mbps(2), ProbeKind::Initial);
    assert!(
        !estimator.probe_start(config, base),
        "should reject probe when at capacity with no stale probes"
    );
    assert_eq!(estimator.states.len(), MAX_ACTIVE_PROBES);

    // Now try adding a probe 6 seconds later - all existing probes are stale
    let later = base + Duration::from_secs(6);
    let config = ProbeClusterConfig::new(101.into(), Bitrate::mbps(2), ProbeKind::Initial);
    assert!(
        estimator.probe_start(config, later),
        "should accept probe after cleaning stale ones"
    );
    // All old probes should be cleaned, leaving only the new one
    assert_eq!(estimator.states.len(), 1);
}

#[test]
fn non_stale_probes_preserved_during_cleanup() {
    let mut estimator = ProbeEstimator::new();
    let base = Instant::now();

    // Add some old probes
    for i in 0..15_u64 {
        let config = ProbeClusterConfig::new(i.into(), Bitrate::mbps(2), ProbeKind::Initial);
        estimator.probe_start(config, base);
    }

    // Add some newer probes (3 seconds later, still within threshold)
    let mid = base + Duration::from_secs(3);
    for i in 15..MAX_ACTIVE_PROBES {
        let config =
            ProbeClusterConfig::new((i as u64).into(), Bitrate::mbps(2), ProbeKind::Initial);
        estimator.probe_start(config, mid);
    }
    assert_eq!(estimator.states.len(), MAX_ACTIVE_PROBES);

    // Try to add at 6 seconds - old probes are stale, mid probes are not
    let later = base + Duration::from_secs(6);
    let config = ProbeClusterConfig::new(100.into(), Bitrate::mbps(2), ProbeKind::Initial);
    assert!(
        estimator.probe_start(config, later),
        "should accept after cleaning only stale probes"
    );
    // 15 old probes removed, 5 mid probes kept, 1 new probe added = 6
    assert_eq!(estimator.states.len(), 6);
}
