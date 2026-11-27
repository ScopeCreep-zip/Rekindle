use super::*;

#[test]
fn no_backoff_before_any_feedback() {
    let mut b = RttBackoff::default();
    let t0 = Instant::now();
    b.on_packet_sent(t0 + Duration::from_secs(10));
    assert_eq!(
        b.backoff(t0 + Duration::from_secs(10), Bitrate::kbps(800)),
        None
    );
}

#[test]
fn sending_without_feedback_cuts_once_a_second() {
    let mut b = RttBackoff::default();
    let t0 = Instant::now();
    b.update_propagation_rtt(t0, Duration::from_millis(300));
    // Still inside the limit at 2.5 s of silence.
    b.on_packet_sent(t0 + Duration::from_millis(2_500));
    assert_eq!(
        b.backoff(t0 + Duration::from_millis(2_500), Bitrate::kbps(800)),
        None
    );
    // 2.8 s + 0.3 s > 3 s.
    let t1 = t0 + Duration::from_millis(2_800);
    b.on_packet_sent(t1);
    assert_eq!(b.backoff(t1, Bitrate::kbps(800)), Some(Bitrate::kbps(640)));
    // Not again within a second.
    b.on_packet_sent(t1 + Duration::from_millis(500));
    assert_eq!(
        b.backoff(t1 + Duration::from_millis(500), Bitrate::kbps(640)),
        None
    );
    let t2 = t1 + Duration::from_secs(1);
    b.on_packet_sent(t2);
    assert_eq!(b.backoff(t2, Bitrate::kbps(640)), Some(Bitrate::kbps(512)));
}

#[test]
fn fresh_feedback_stops_the_backoff() {
    let mut b = RttBackoff::default();
    let t0 = Instant::now();
    b.update_propagation_rtt(t0, Duration::from_millis(300));
    let t1 = t0 + Duration::from_secs(4);
    b.on_packet_sent(t1);
    assert!(b.backoff(t1, Bitrate::kbps(800)).is_some());
    b.update_propagation_rtt(t1, Duration::from_millis(300));
    let t2 = t1 + Duration::from_secs(2);
    assert_eq!(b.backoff(t2, Bitrate::kbps(640)), None);
}

#[test]
fn never_below_the_floor() {
    let mut b = RttBackoff::default();
    let t0 = Instant::now();
    b.update_propagation_rtt(t0, Duration::from_secs(4));
    assert_eq!(b.backoff(t0, Bitrate::kbps(6)), Some(Bitrate::kbps(5)));
}
