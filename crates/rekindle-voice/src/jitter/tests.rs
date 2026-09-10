//! Unit + regression tests for the adaptive jitter buffer.
//!
//! Split out to keep `mod.rs` under the file-size ceiling; this is the
//! `#[cfg(test)] mod tests;` sibling of `jitter/mod.rs`.

use super::*;

fn make_packet(seq: u32) -> VoicePacket {
    VoicePacket {
        sender_key: vec![0; 32],
        sequence: seq,
        timestamp: u64::from(seq) * 20,
        audio_data: vec![0; 160],
        mek_generation: 0,
        signature: Vec::new(),
    }
}

#[test]
fn base_from_config_not_hardcoded() {
    // The target starts at the config base (no 200 ms floor), then
    // MIN-clamps via recompute only once jitter is observed.
    assert_eq!(JitterBuffer::new(40).target_delay_ms(), 40);
    assert_eq!(JitterBuffer::new(80).target_delay_ms(), 80);
}

#[test]
fn adaptive_target_grows_under_jitter_and_clamps() {
    let mut jb = JitterBuffer::new(40);
    // Arrival cadence diverging from the 20 ms sender cadence is
    // jitter: arrivals 50 ms apart against 20 ms of sender time is a
    // steady 30 ms |Δtransit|. Sequence advances by one so the walk
    // sees a clean stream and not loss.
    let mut sender_ms = 0u64;
    let mut arrival_ms = 0u64;
    for seq in 0..50u32 {
        jb.observe_arrival(seq, sender_ms, arrival_ms, true);
        sender_ms += 20;
        arrival_ms += 50;
    }
    let t = jb.target_delay_ms();
    // base 40 + ~2.5×30 ≈ 115, clamped to MAX 120.
    assert!(t > 40 && t <= JITTER_MAX_MS, "grew + clamped: {t}");
    assert!(
        jb.reception_metrics().jitter_ms >= 25,
        "the reported estimate is the same one the target used"
    );

    // Huge jitter clamps at MAX.
    for seq in 50..100u32 {
        jb.observe_arrival(seq, sender_ms, arrival_ms, true);
        sender_ms += 20;
        arrival_ms += 1000;
    }
    assert_eq!(jb.target_delay_ms(), JITTER_MAX_MS);
}

#[test]
fn late_drops_grow_clean_windows_shrink_never_below_base() {
    let mut jb = JitterBuffer::new(60);
    // Late drops bump the target up a step.
    jb.note_window_health(3);
    assert!(jb.target_delay_ms() > 60, "late drops grow target");
    let grown = jb.target_delay_ms();
    // Two clean windows are required before any shrink; with zero
    // observed jitter the EWMA floor is the base, so it slow-shrinks
    // toward 60 but never below.
    jb.note_window_health(0); // clean_windows = 1, no shrink yet
    assert_eq!(jb.target_delay_ms(), grown, "one clean window: no shrink");
    jb.note_window_health(0); // clean_windows = 2, shrink one step
    assert!(jb.target_delay_ms() < grown && jb.target_delay_ms() >= 60);
    for _ in 0..10 {
        jb.note_window_health(0);
    }
    assert_eq!(jb.target_delay_ms(), 60, "shrinks to base, not below");
}

#[test]
fn test_in_order_playback() {
    // Use 0ms target delay to skip initial fill (unit test only).
    // Instant arrival/playout: a fixed 0 ms clock replicates the
    // old `Instant::now()` fast-loop behavior exactly.
    let mut jb = JitterBuffer::new(0);
    jb.push(make_packet(0), 0);
    jb.push(make_packet(1), 0);
    jb.push(make_packet(2), 0);

    assert_eq!(jb.pop(0).unwrap().sequence, 0);
    assert_eq!(jb.pop(0).unwrap().sequence, 1);
    assert_eq!(jb.pop(0).unwrap().sequence, 2);
    assert!(jb.pop(0).is_none());
}

#[test]
fn test_out_of_order() {
    let mut jb = JitterBuffer::new(0);
    jb.push(make_packet(2), 0);
    jb.push(make_packet(0), 0);
    jb.push(make_packet(1), 0);

    assert_eq!(jb.pop(0).unwrap().sequence, 0);
    assert_eq!(jb.pop(0).unwrap().sequence, 1);
    assert_eq!(jb.pop(0).unwrap().sequence, 2);
}

#[test]
fn test_late_packet_dropped() {
    let mut jb = JitterBuffer::new(0);
    jb.push(make_packet(0), 0);
    jb.pop(0); // consume 0, next_playback_seq = 1

    jb.push(make_packet(0), 0); // late, should be dropped
    assert_eq!(jb.depth(), 0);
    assert_eq!(jb.take_drops(), (0, 1), "late drop counted");
    assert_eq!(jb.take_drops(), (0, 0), "take_drops resets");
}

#[test]
fn gap_jump_resumes_from_oldest_buffered() {
    // The adaptive MIN floor is 40 ms → minimum jump window is 2
    // ticks (40/20). Instant-arrival test packets vs the 20 ms
    // sender cadence register as jitter, pinning the target at the
    // 40 ms floor. So it takes 2 noted misses before the gap jumps.
    let mut jb = JitterBuffer::new(0);
    jb.push(make_packet(0), 0);
    jb.push(make_packet(1), 0);
    // seq 2 lost; 3 and 4 arrive.
    jb.push(make_packet(3), 0);
    jb.push(make_packet(4), 0);
    assert_eq!(jb.pop(0).unwrap().sequence, 0);
    assert_eq!(jb.pop(0).unwrap().sequence, 1);
    assert!(jb.pop(0).is_none(), "seq 2 is lost");
    assert!(
        jb.note_miss_and_maybe_jump().is_none(),
        "within jitter window"
    );
    let jumped = jb.note_miss_and_maybe_jump().unwrap();
    assert_eq!(jumped.sequence, 3, "resumes from oldest buffered");
    assert_eq!(jb.pop(0).unwrap().sequence, 4, "stream continues in order");
}

#[test]
fn gap_jump_waits_out_the_jitter_window() {
    // 60ms target → 3 ticks of grace before jumping.
    let mut jb = JitterBuffer::new(60);
    for seq in 0..3 {
        jb.push(make_packet(seq), 0);
    }
    while jb.pop(0).is_some() {}
    jb.push(make_packet(5), 0); // seq 3 + 4 lost
    assert!(jb.note_miss_and_maybe_jump().is_none(), "miss 1: wait");
    assert!(jb.note_miss_and_maybe_jump().is_none(), "miss 2: wait");
    let jumped = jb.note_miss_and_maybe_jump().unwrap();
    assert_eq!(jumped.sequence, 5, "miss 3: jump");
}

#[test]
fn fec_advance_unsticks_playback_position() {
    let mut jb = JitterBuffer::new(0);
    jb.push(make_packet(0), 0);
    assert_eq!(jb.pop(0).unwrap().sequence, 0);
    // seq 1 lost, 2 buffered → FEC peek sees 2's payload.
    jb.push(make_packet(2), 0);
    assert!(jb.pop(0).is_none());
    // Mirror the real decode cycle: a miss goes through
    // `note_miss_and_maybe_jump` (which counts the miss) before the
    // FEC peek. With no reordering seen, FEC is offered on that
    // first miss — the fast single-loss concealment path.
    assert!(
        jb.note_miss_and_maybe_jump().is_none(),
        "within window: no jump yet"
    );
    assert!(jb.peek_next_audio_data().is_some());
    jb.advance_after_fec();
    assert_eq!(
        jb.pop(0).unwrap().sequence,
        2,
        "next pop returns the packet FEC peeked, not another conceal"
    );
}

#[test]
fn lost_packet_no_longer_stalls_into_endless_overflow() {
    // Regression for the live failure: one lost packet pinned
    // playback while every later arrival was trimmed as overflow
    // (rx_overflow_drops in the hundreds per 5 s, dead audio).
    let mut jb = JitterBuffer::new(0);
    jb.push(make_packet(0), 0);
    assert_eq!(jb.pop(0).unwrap().sequence, 0);
    // seq 1 lost; a long run of later packets arrives.
    for seq in 2..80 {
        jb.push(make_packet(seq), 0);
    }
    // Playout tick: miss → jump → stream drains in order. The 40 ms
    // adaptive floor makes the jump window 2 ticks, so the gap is
    // declared on the second noted miss.
    assert!(jb.pop(0).is_none());
    assert!(
        jb.note_miss_and_maybe_jump().is_none(),
        "within jitter window"
    );
    assert!(jb.note_miss_and_maybe_jump().is_some());
    let mut drained = 1;
    while jb.pop(0).is_some() {
        drained += 1;
    }
    let (overflow, _) = jb.take_drops();
    // 78 pushed after the gap; max_packets=50 bounds the buffer, so
    // pre-jump trims are expected — but everything still buffered
    // plays out instead of being discarded one-per-arrival forever.
    assert_eq!(
        drained + overflow,
        78,
        "every packet played or trimmed once"
    );
    assert!(drained >= 50, "the surviving window drains fully");
}

#[test]
fn test_overflow_trim_counted() {
    let mut jb = JitterBuffer::new(0);
    for seq in 0..60 {
        jb.push(make_packet(seq), 0);
    }
    // max_packets = 50 → ten packets trimmed.
    assert_eq!(jb.depth(), 50);
    let (overflow, late) = jb.take_drops();
    assert_eq!(overflow, 10, "overflow trim counted");
    assert_eq!(late, 0);
}

/// Regression guard for the reorder-aware jitter buffer, born from
/// the live "receiver discards ~86% of arriving packets, loss ~0"
/// failure under the media route's `Sequencing::PreferUnordered`.
///
/// Drives the buffer through the REAL receive-loop decode cycle
/// (`receive_loop::decode_all_participants`: pop → jump → FEC → PLC)
/// against a controlled reordered arrival schedule, and asserts the
/// resulting RFC 3611 discard rate. The mechanism it locks in: both
/// the gap-jump (`note_miss_and_maybe_jump`) and the FEC-advance
/// (`peek_next_audio_data` → `advance_after_fec`) used to push
/// `next_playback_seq` past a reordered-but-not-lost packet, so when
/// that packet arrived it was `seq < next_playback_seq` and dropped
/// as a *discard* (not a loss). Both are now gated by the observed
/// `reorder_span`, so the buffer waits reordered packets out instead
/// of skipping them.
///
/// PreferUnordered is modeled as a reorder distance `R`: within each
/// block of `R` packets the delivery order is reversed, so the
/// packet the player needs next (lowest seq in the block) is
/// consistently the LAST of its block to arrive — `(R-1)·20 ms`
/// after the block began arriving. Before the fix that defeated the
/// 2-tick starting window (83% discard at R=6, 90% at R=10); after
/// it, both collapse to a small startup residue.
#[test]
#[allow(
    clippy::print_stderr,
    reason = "diagnostic reproduction; the discard rate IS its output, read via cargo test --nocapture"
)]
fn reorder_aware_jump_absorbs_reordering() {
    /// Sender emits seq 0..N, one 20 ms frame each.
    const N: u32 = 600;
    /// Frame / tick granularity in milliseconds.
    const FRAME_MS: u64 = 20;

    /// Run the full decode cycle against a block-reversed arrival
    /// schedule with reorder distance `reorder`, returning the final
    /// reception metrics plus the accumulated `(overflow, late)`
    /// drop split.
    fn run_reorder(reorder: u32) -> (ReceptionMetrics, (u64, u64)) {
        // Voice/media base target (JITTER_MIN_MS floor).
        let mut jb = JitterBuffer::new(40);

        // Build the delivery schedule: (seq, arrival_ms), already in
        // arrival order. Within each block of `r`, deliver seqs from
        // high to low against ascending arrival slots, so the lowest
        // seq of the block arrives last while arrival_ms stays
        // globally monotonic (low, realistic interarrival spacing).
        let r = reorder.max(1);
        let mut schedule: Vec<(u32, u64)> = Vec::new();
        let mut block_start = 0u32;
        while block_start < N {
            let block_end = (block_start + r).min(N);
            let mut arrival_pos = block_start;
            let mut seq = block_end;
            while seq > block_start {
                seq -= 1;
                schedule.push((seq, u64::from(arrival_pos) * FRAME_MS));
                arrival_pos += 1;
            }
            block_start = block_end;
        }

        // Drive wall time in 20 ms ticks: push everything that has
        // arrived by `t` (in arrival order), then run ONE decode
        // cycle at now_ms = t — mirroring decode_all_participants.
        let end_ms = u64::from(N) * FRAME_MS * 2; // margin to drain
        let mut pushed = 0usize;
        let mut t = 0u64;
        while t <= end_ms {
            while pushed < schedule.len() && schedule[pushed].1 <= t {
                let (seq, arrival_ms) = schedule[pushed];
                jb.push(make_packet(seq), arrival_ms);
                pushed += 1;
            }
            match jb.pop(t) {
                Some(_) => { /* played a real frame */ }
                None => {
                    if jb.note_miss_and_maybe_jump().is_some() {
                        // gap jump: resumed from oldest buffered
                    } else if jb.peek_next_audio_data().is_some() {
                        jb.advance_after_fec(); // FEC recovery
                    } else {
                        // PLC conceal — no buffer state change
                    }
                }
            }
            t += FRAME_MS;
        }

        (jb.reception_metrics(), jb.take_drops())
    }

    let pct = |q8: u8| u32::from(q8) * 100 / 256;
    let report = |label: &str, m: &ReceptionMetrics, drops: (u64, u64)| {
        eprintln!(
            "{label}: discard={} q8 (~{}%)  loss={} q8 (~{}%)  jitter={}ms  \
             overflow_drops={}  late_drops={}  expected={}  received={}",
            m.discard_rate_q8,
            pct(m.discard_rate_q8),
            m.loss_rate_q8,
            pct(m.loss_rate_q8),
            m.jitter_ms,
            drops.0,
            drops.1,
            m.packets_expected,
            m.packets_received,
        );
    };

    let (mild, mild_drops) = run_reorder(2);
    let (heavy, heavy_drops) = run_reorder(6);
    let (severe, severe_drops) = run_reorder(10);

    eprintln!("--- reordered-delivery discard reproduction (media PreferUnordered) ---");
    report("R=2  (mild, within 2-tick jump window)", &mild, mild_drops);
    report("R=6  (heavy)", &heavy, heavy_drops);
    report("R=10 (severe)", &severe, severe_drops);

    // Regression guard for the reorder-aware gap-jump window. Before
    // the fix this test showed ~83 % discard at R=6 and ~90 % at
    // R=10 (the jump skipping reordered-but-not-lost packets). Now
    // both R=6 and R=10 sit inside `REORDER_SPAN_MAX_PACKETS` (15),
    // so the jump waits them out: discard collapses to the small
    // startup residue (the first block, before the buffer has seen
    // the route reorder once). Loss stays 0 — nothing was ever
    // actually lost, only reordered.
    assert_eq!(
        heavy.loss_rate_q8, 0,
        "reorder is not network loss — nothing was actually lost"
    );
    assert!(
        pct(heavy.discard_rate_q8) < 5,
        "reorder-aware jump must absorb R=6 reordering: discard={}% (was ~83%)",
        pct(heavy.discard_rate_q8),
    );
    assert!(
        pct(severe.discard_rate_q8) < 5,
        "R=10 is within the {REORDER_SPAN_MAX_PACKETS}-packet cap and must also \
         be absorbed: discard={}% (was ~90%)",
        pct(severe.discard_rate_q8),
    );
}
