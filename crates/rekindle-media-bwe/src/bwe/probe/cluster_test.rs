use super::*;

#[test]
fn low_rate_padding_reaches_minimum_packet_count() {
    let now = Instant::now();
    let config = ProbeClusterConfig::new(0.into(), Bitrate::kbps(10), ProbeKind::Initial);
    let mut state = ProbeClusterState::new(config);
    for i in 0..config.min_packet_count() {
        let at = if i == 0 { now } else { state.next_probe_time() };
        let size = state.next_packet(at).expect("minimum packets still needed");
        assert!(size <= MAX_PADDING_PACKET_SIZE);
        state.record_packet(at, size);
    }
    assert!(state.is_complete(state.next_probe_time()));
}

// Test helper to directly set state for testing
impl ProbeClusterState {
    fn test_set_state(&mut self, bytes_sent: DataSize, packets_sent: usize, now: Instant) {
        self.bytes_sent = bytes_sent;
        self.packets_sent = packets_sent;
        if self.started_at.is_none() {
            self.started_at = Some(now);
        }
        self.last_packet_at = Some(now);
    }
}

#[test]
fn probe_cluster_not_complete_before_start() {
    let now = Instant::now();
    let config = ProbeClusterConfig::new(1.into(), Bitrate::mbps(3), ProbeKind::Initial);
    let state = ProbeClusterState::new(config);

    assert!(!state.is_complete(now));
}

#[test]
fn probe_cluster_not_complete_after_packets_but_not_bytes() {
    let now = Instant::now();
    let config = ProbeClusterConfig::new(1.into(), Bitrate::mbps(3), ProbeKind::Initial);
    // target_bytes = 3 Mbps * 15ms = 45,000 bits = 5,625 bytes
    let mut state = ProbeClusterState::new(config);

    // Send 5 packets (meets packet count) but only small packets (doesn't meet bytes)
    // 5 * 100 = 500 bytes, which is < 5,625 bytes
    state.test_set_state(DataSize::bytes(500), 5, now);

    // Not complete: packets met but bytes not met
    assert!(!state.is_complete(now));
}

#[test]
fn probe_cluster_not_complete_after_bytes_but_not_packets() {
    let now = Instant::now();
    let config = ProbeClusterConfig::new(1.into(), Bitrate::mbps(3), ProbeKind::Initial);
    // target_bytes = 3 Mbps * 15ms = 45,000 bits = 5,625 bytes
    let mut state = ProbeClusterState::new(config);

    // Send 2 large packets (meets bytes) but not enough packets
    // 2 * 3000 = 6000 bytes > 5,625 bytes, but only 2 packets < 5 packets
    state.test_set_state(DataSize::bytes(6000), 2, now);

    // Not complete: bytes met but packets not met
    assert!(!state.is_complete(now));
}

#[test]
fn probe_cluster_complete_when_both_criteria_met() {
    let now = Instant::now();
    let config = ProbeClusterConfig::new(1.into(), Bitrate::mbps(3), ProbeKind::Initial);
    // target_bytes = 3 Mbps * 15ms = 45,000 bits = 5,625 bytes
    let mut state = ProbeClusterState::new(config);

    // Send 5 packets of 1200 bytes each = 6000 bytes
    // Meets both: 6000 >= 5625 bytes AND 5 >= 5 packets
    state.test_set_state(DataSize::bytes(6000), 5, now);

    // Complete: both bytes and packets met, even at same instant
    assert!(state.is_complete(now));
}

#[test]
fn probe_cluster_complete_even_with_zero_duration() {
    let now = Instant::now();
    let config = ProbeClusterConfig::new(1.into(), Bitrate::mbps(3), ProbeKind::Initial);
    let mut state = ProbeClusterState::new(config);

    // Send all packets at exactly the same instant (duration = 0)
    state.test_set_state(DataSize::bytes(6000), 5, now);

    // Should still complete (this is the bug fix - no duration requirement)
    assert!(state.is_complete(now));
}

#[test]
fn probe_cluster_tracks_bytes_and_packets() {
    let now = Instant::now();
    let config = ProbeClusterConfig::new(1.into(), Bitrate::mbps(3), ProbeKind::Initial);
    let mut state = ProbeClusterState::new(config);

    state.test_set_state(DataSize::bytes(1200), 1, now);
    assert_eq!(state.bytes_sent, DataSize::bytes(1200));
    assert_eq!(state.packets_sent, 1);

    state.test_set_state(DataSize::bytes(2200), 2, now + Duration::from_millis(1));
    assert_eq!(state.bytes_sent, DataSize::bytes(2200));
    assert_eq!(state.packets_sent, 2);
}

#[test]
fn min_probe_delta_is_enforced_in_next_probe_time() {
    let now = Instant::now();
    let config = ProbeClusterConfig::new(1.into(), Bitrate::mbps(3), ProbeKind::Initial)
        .with_min_probe_delta(Duration::from_millis(20));
    let mut state = ProbeClusterState::new(config);
    assert_eq!(state.config().min_probe_delta(), Duration::from_millis(20));

    // Start a burst at `now`, then verify that a second burst is blocked until >= now+20ms.
    assert!(state.next_packet(now).is_some());
    assert!(state.next_packet(now + Duration::from_millis(19)).is_none());
    assert!(state.next_packet(now + Duration::from_millis(20)).is_some());
    let next = state.last_padding_at;
    assert!(
        next >= now + Duration::from_millis(20),
        "next_probe_time {next:?} must be >= now + min_probe_delta"
    );
}
