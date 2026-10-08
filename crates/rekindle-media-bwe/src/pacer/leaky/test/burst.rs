//! Burst-size tests (str0m `src/pacer/leaky.rs`, `mod test`).

use super::*;

#[test]
fn test_max_burst_size_at_high_bitrate() {
    // At 50 Mbps a full PACING interval is 250 kB, which is well above MAX_BURST_SIZE.
    let (burst, next_poll) = measure_burst(Bitrate::mbps(50), 1200, 500);

    assert!(
        burst <= MAX_BURST_SIZE + DataSize::bytes(1200),
        "Burst of {burst} should not exceed MAX_BURST_SIZE by more than one packet"
    );
    assert!(
        burst > MAX_BURST_SIZE,
        "Burst of {burst} should fill up to MAX_BURST_SIZE"
    );
    // 63 kB at 50 Mbps drains in ~10 ms.
    assert!(
        next_poll >= duration_ms(10) && next_poll < PACING,
        "Pacer should wait for the burst to drain, got {next_poll:?}"
    );

    // When packet sizes divide the limit exactly, stop at the limit rather than
    // sending another packet.
    let (burst, _) = measure_burst(Bitrate::mbps(50), 1000, 500);
    assert_eq!(burst, MAX_BURST_SIZE);
}

#[test]
fn test_burst_size_unchanged_at_low_bitrate() {
    // At 10 Mbps a PACING interval is 50 kB, below MAX_BURST_SIZE, so PACING still governs
    // the burst: debt may grow to 50 kB, then the pacer waits.
    let (burst, next_poll) = measure_burst(Bitrate::mbps(10), 1000, 500);
    assert_eq!(burst, DataSize::bytes(50_000));
    // Nothing is sent before the regular PACING tick.
    assert_eq!(next_poll, PACING);

    // At 1 Mbps a PACING interval is 5 kB. Queue less here so the queue drain logic
    // doesn't raise the adjusted bitrate.
    let (burst, next_poll) = measure_burst(Bitrate::mbps(1), 1000, 50);
    assert_eq!(burst, DataSize::bytes(5_000));
    // Nothing is sent before the regular PACING tick.
    assert_eq!(next_poll, PACING);
}

/// Queue `count` video packets of `size` bytes at once and poll the pacer until it
/// stops releasing packets without time moving forward.
///
/// Returns the number of bytes released in that burst and how long after the burst the
/// pacer wants to be polled again.
fn measure_burst(bitrate: Bitrate, size: usize, count: u16) -> (DataSize, Duration) {
    let now = Instant::now();
    let mut queue = Queue::default();
    let mut pacer = LeakyBucketPacer::new(bitrate);
    handle_timeout_noisy(&mut pacer, &mut queue, now);

    let at = now + duration_ms(1);
    for seq_no in 0..count {
        enqueue_packet_noisy(&mut pacer, &mut queue, seq_no, size, PacketKind::Video, at);
    }

    let mut burst = DataSize::ZERO;
    while let Some((qid, _)) = pacer.poll_queue() {
        let packet = queue.next_packet().unwrap();
        pacer.register_send(at, DataSize::from(packet.size()), qid);
        queue.register_send(qid, at);
        handle_timeout_noisy(&mut pacer, &mut queue, at);
        burst += DataSize::from(packet.size());
    }

    assert!(!queue.is_empty(), "The burst should be cut short by pacing");

    let next_poll = pacer.poll_timeout().0.unwrap();
    (burst, next_poll - at)
}
