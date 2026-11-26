use super::*;

/// Feed `n` packets at a steady 20 ms cadence, skipping any
/// sequence in `drop`.
fn run(n: u32, drop: &[u32]) -> ReceptionTracker {
    let mut t = ReceptionTracker::new(20);
    for seq in 0..n {
        if drop.contains(&seq) {
            continue;
        }
        let ms = u64::from(seq) * 20;
        t.on_packet(seq, ms, ms, true);
    }
    t
}

#[test]
fn clean_stream_has_no_loss_and_no_jitter() {
    let m = run(100, &[]).metrics();
    assert_eq!(m.loss_rate_q8, 0);
    assert_eq!(m.jitter_ms, 0);
    assert_eq!(m.packets_received, 100);
    assert_eq!(m.packets_expected, 100);
}

/// The point of the whole module: the same loss rate, arriving two
/// different ways, must not look the same.
#[test]
fn bursty_and_uniform_loss_are_distinguishable_at_equal_rate() {
    // 10 lost of 200, spread out. Deliberately not `% 20 == 19`:
    // that drops sequence 199, the last one, so `highest_seq`
    // becomes 198 and the denominator shrinks — the two cases would
    // then differ in loss rate for a reason that has nothing to do
    // with burstiness.
    let uniform: Vec<u32> = (0..200).filter(|s| s % 20 == 10).collect();
    // 10 lost of 200, all together.
    let bursty: Vec<u32> = (100..110).collect();

    let u = run(200, &uniform).metrics();
    let b = run(200, &bursty).metrics();

    assert_eq!(u.loss_rate_q8, b.loss_rate_q8, "same loss rate by design");
    assert!(
        b.burst_duration_ms > u.burst_duration_ms,
        "the clustered outage must show a longer mean burst: {} vs {}",
        b.burst_duration_ms,
        u.burst_duration_ms
    );
    assert!(
        b.burst_density_q8 > u.burst_density_q8,
        "and a denser one: {} vs {}",
        b.burst_density_q8,
        u.burst_density_q8
    );
}

#[test]
fn a_run_of_gmin_clean_packets_closes_the_burst() {
    // One loss, then well over Gmin clean packets, then another.
    let t = run(80, &[10, 60]);
    let m = t.metrics();
    assert_eq!(m.packets_received, 78);
    assert!(
        m.gap_duration_ms > 0,
        "the clean stretch between the two losses is a gap"
    );
}

#[test]
fn discards_are_counted_apart_from_loss() {
    let mut t = ReceptionTracker::new(20);
    for seq in 0..50u32 {
        let ms = u64::from(seq) * 20;
        // Every 10th packet arrives too late to play.
        t.on_packet(seq, ms, ms, seq % 10 != 0);
    }
    let m = t.metrics();
    assert_eq!(m.loss_rate_q8, 0, "nothing was lost — everything arrived");
    assert!(m.discard_rate_q8 > 0, "but some of it was unusable");
}

#[test]
fn jitter_tracks_variable_arrival() {
    let mut t = ReceptionTracker::new(20);
    for seq in 0..100u32 {
        let sent = u64::from(seq) * 20;
        // Alternate a 30 ms extra delay — a 30 ms swing in transit.
        let arrived = sent + if seq % 2 == 0 { 0 } else { 30 };
        t.on_packet(seq, sent, arrived, true);
    }
    let m = t.metrics();
    assert!(
        m.jitter_ms >= 20 && m.jitter_ms <= 30,
        "a 30 ms alternating swing should settle near it, got {}",
        m.jitter_ms
    );
}

#[test]
fn reordering_is_not_reported_as_mass_loss() {
    let mut t = ReceptionTracker::new(20);
    // A late straggler arriving after its successors must not be
    // read as ~4 billion missing packets via wrapping subtraction.
    for seq in [0u32, 1, 2, 5, 3, 4, 6] {
        let ms = u64::from(seq) * 20;
        t.on_packet(seq, ms, ms, true);
    }
    let m = t.metrics();
    assert_eq!(m.packets_received, 7);
    assert!(m.loss_rate_q8 == 0, "nothing was actually lost");
}

#[test]
fn empty_stream_reports_zero_not_total_loss() {
    let m = ReceptionTracker::new(20).metrics();
    assert_eq!(m.loss_rate_q8, 0);
    assert_eq!(m.packets_expected, 0);
}

/// Plan E4.3.0 — relative delay is each packet's transit above the
/// window floor, so a constant clock offset between the machines
/// cancels and a queue building on the route shows in the tail.
#[test]
fn relative_delay_percentiles_ignore_clock_offset() {
    let mut t = ReceptionTracker::new(20);
    // Sender clock 10 s ahead of ours; 100 packets on time, then 5
    // queued 200 ms behind.
    for seq in 0..105u32 {
        let sent = 10_000_000 + u64::from(seq) * 20;
        let queued = if seq >= 100 { 200 } else { 0 };
        t.on_packet(seq, sent, u64::from(seq) * 20 + 40 + queued, true);
    }
    let m = t.metrics();
    assert_eq!(m.delay_p50_ms, 0);
    assert_eq!(m.delay_p95_ms, 0);
    // Ten more queued packets push them past the 95th percentile.
    for seq in 105..115u32 {
        let sent = 10_000_000 + u64::from(seq) * 20;
        t.on_packet(seq, sent, u64::from(seq) * 20 + 240, true);
    }
    let m = t.metrics();
    assert_eq!(m.delay_p50_ms, 0);
    assert_eq!(m.delay_p95_ms, 200);
}

#[test]
fn delay_window_forgets_old_packets() {
    let mut t = ReceptionTracker::new(20);
    // A slow start, then a full window on time.
    for seq in 0..10u32 {
        t.on_packet(seq, u64::from(seq) * 20, u64::from(seq) * 20 + 300, true);
    }
    for seq in 10..(10 + u32::try_from(DELAY_WINDOW_PACKETS).unwrap()) {
        t.on_packet(seq, u64::from(seq) * 20, u64::from(seq) * 20 + 50, true);
    }
    assert_eq!(t.metrics().delay_p95_ms, 0);
}
