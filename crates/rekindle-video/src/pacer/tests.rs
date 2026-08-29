use super::*;
use crate::fragment::FRAGMENT_PAYLOAD_LIMIT;
use proptest::prelude::*;

fn fragment_envelope(payload_len: usize) -> CommunityEnvelope {
    CommunityEnvelope::Control(ControlPayload::VideoFragment {
        channel_id: "ch".into(),
        stream_id: [1; 16],
        frame_seq: 0,
        frag_index: 0,
        frag_total: 1,
        keyframe: false,
        codec: rekindle_types::video::Codec::Vp9,
        timestamp: 0,
        mek_generation: 0,
        payload: vec![0; payload_len],
        signature: Vec::new(),
    })
}

fn parity_envelope(payload_len: usize) -> CommunityEnvelope {
    CommunityEnvelope::Control(ControlPayload::VideoParityFragment {
        channel_id: "ch".into(),
        stream_id: [1; 16],
        frame_seq: 0,
        parity_index: 0,
        parity_total: 1,
        data_count: 1,
        codec: rekindle_types::video::Codec::Vp9,
        frame_len: 0,
        timestamp: 0,
        mek_generation: 0,
        payload: vec![0; payload_len],
        signature: Vec::new(),
    })
}

/// Keyframe with `data` data shards + `parity` parity shards, all
/// `frag_bytes` — the canonical post-FEC shape.
fn fec_keyframe(seq: u32, data: usize, parity: usize, frag_bytes: usize, t: u64) -> PacedFrame {
    let mut envelopes: Vec<CommunityEnvelope> =
        (0..data).map(|_| fragment_envelope(frag_bytes)).collect();
    envelopes.extend((0..parity).map(|_| parity_envelope(frag_bytes)));
    PacedFrame {
        community_id: "c".into(),
        channel_id: "ch".into(),
        stream_id: [1; 16],
        frame_seq: seq,
        keyframe: true,
        envelopes,
        bytes: data * frag_bytes,
        enqueued_ms: t,
    }
}

fn frame(seq: u32, keyframe: bool, fragments: usize, frag_bytes: usize, t: u64) -> PacedFrame {
    // (u32 seq keeps call sites cast-free under pedantic lints.)
    PacedFrame {
        community_id: "c".into(),
        channel_id: "ch".into(),
        stream_id: [1; 16],
        frame_seq: seq,
        keyframe,
        envelopes: (0..fragments)
            .map(|_| fragment_envelope(frag_bytes))
            .collect(),
        bytes: fragments * frag_bytes,
        enqueued_ms: t,
    }
}

#[test]
fn fragments_released_in_order_at_rate() {
    // 350 kbps = 43_750 B/s wire. A 3×4 KiB frame (≈16.5 KiB wire):
    // the initial bucket (≈10.9 KiB) affords the first fragment,
    // the rest trickles.
    let mut p = VideoPacer::new(350);
    p.enqueue(frame(1, true, 3, 4_096, 0), 0);
    let first = p.poll(0);
    assert!(!first.is_empty(), "initial bucket affords ≥1 fragment");
    let mut total = first.len();
    let mut t = 0;
    while total < 3 {
        t += p.next_poll_in_ms(t).max(1);
        total += p.poll(t).len();
        assert!(t < 5_000, "3 fragments must release within 5s at 350kbps");
    }
    assert_eq!(p.stats().sent_fragments, 3);
}

#[test]
fn in_flight_frame_always_completes() {
    let mut p = VideoPacer::new(350);
    p.enqueue(frame(1, false, 4, 4_096, 0), 0);
    // Start releasing frame 1 (cursor > 0)…
    let released = p.poll(0);
    assert!(!released.is_empty() && released.len() < 4);
    // …then saturate the queue. The in-flight frame must survive.
    for seq in 2..40 {
        p.enqueue(frame(seq, false, 1, 1_000, 1), 1);
    }
    let mut t: u64 = 1;
    let mut got = released.len();
    for _ in 0..200 {
        t += p.next_poll_in_ms(t).max(1);
        got += p
            .poll(t)
            .iter()
            .filter(|(_, _, e)| envelope_data_payload_bytes(e) == 4_096)
            .count();
        if got == 4 {
            break;
        }
    }
    assert_eq!(got, 4, "every fragment of the in-flight frame ships");
}

#[test]
fn expiry_sheds_stale_non_keyframes_only() {
    let mut p = VideoPacer::new(100);
    p.enqueue(frame(1, true, 1, 4_096, 0), 0);
    p.enqueue(frame(2, false, 1, 4_096, 0), 0);
    p.enqueue(frame(3, false, 1, 4_096, 0), 0);
    // Far past MAX_QUEUE_AGE_MS: the deltas expire un-sent; the
    // keyframe is exempt — it SHIPS (late) instead of expiring.
    let released = p.poll(MAX_QUEUE_AGE_MS + 1_000);
    let s = p.stats();
    assert_eq!(s.expired_frames, 2, "both deltas expire");
    assert_eq!(
        s.sent_fragments, 1,
        "the keyframe ships late, never expires"
    );
    assert_eq!(released.len(), 1);
}

#[test]
fn payload_share_none_before_first_release() {
    let p = VideoPacer::new(350);
    assert_eq!(p.payload_share_q10(), None);
}

#[test]
fn payload_share_tracks_canonical_mix() {
    // The R4 budget-model mix over one 4 s keyframe interval:
    // 1 keyframe (6 data + 2 parity à 4,096) + 47 deltas (2,500 B).
    // payload = 6×4096 + 47×2500 = 142,076
    // wire    = 8×(4096+1400) + 47×(2500+1400) = 227,268
    // share   = 142,076 × 1024 / 227,268 = 640.18 → 640
    let mut p = VideoPacer::new(100_000); // effectively unmetered
    let mut t = 0u64;
    p.enqueue(fec_keyframe(0, 6, 2, 4_096, t), t);
    let _ = p.poll(t);
    for seq in 1..=47u32 {
        // Drain as we go — 48 queued frames would trip the
        // MAX_QUEUED_FRAMES shed and skew the released mix.
        t += 1;
        p.enqueue(frame(seq, false, 1, 2_500, t), t);
        let _ = p.poll(t);
    }
    assert_eq!(p.stats().queue_depth, 0, "mix fully drained");
    assert_eq!(p.stats().dropped_frames, 0, "nothing shed");
    let share = p.payload_share_q10().expect("released a full window");
    assert!(
        (634..=646).contains(&share),
        "share {share} outside 640±6 — wire/payload accounting drifted"
    );
}

#[test]
fn parity_counts_as_wire_not_payload() {
    // A lone FEC keyframe: parity inflates the wire denominator
    // but never the receiver-countable numerator.
    // share = 24,576 × 1024 / 43,968 = 572.36 → 572
    let mut p = VideoPacer::new(100_000);
    p.enqueue(fec_keyframe(0, 6, 2, 4_096, 0), 0);
    let mut t = 0u64;
    for _ in 0..50 {
        t += 50;
        let _ = p.poll(t);
        if p.stats().queue_depth == 0 {
            break;
        }
    }
    assert_eq!(p.payload_share_q10(), Some(572));
}

#[test]
fn oversized_keyframe_counted_at_intake() {
    // 100 kbps → TTL budget = 100×125×500/1000 = 6,250 B; a 2-shard
    // keyframe costs 2×(4096+1400) = 10,992 B wire → flagged.
    let mut p = VideoPacer::new(100);
    p.enqueue(fec_keyframe(0, 2, 0, 4_096, 0), 0);
    assert_eq!(p.stats().oversized_keyframes, 1);
    // At 750 kbps (46,875 B TTL budget) the same frame is fine.
    let mut p = VideoPacer::new(750);
    p.enqueue(fec_keyframe(0, 2, 0, 4_096, 0), 0);
    assert_eq!(p.stats().oversized_keyframes, 0);
}

#[test]
fn keyframe_burst_starves_deltas_at_low_rate_only() {
    // The R4 forbidden band: a 24 KiB keyframe ≈ 44 KiB wire takes
    // ~1 s to drain at 350 kbps — deltas arriving behind it at
    // 12 fps age past the 500 ms TTL. At 750 kbps the drain fits
    // inside the TTL and every delta ships.
    for (rate_kbps, min_expired, max_expired) in [(350u32, 5u64, u64::MAX), (750, 0, 0)] {
        let mut p = VideoPacer::new(rate_kbps);
        // Burn most of the initial bucket burst so the keyframe
        // meets a steady-state bucket, not a full one.
        p.enqueue(frame(0, false, 1, 4_000, 0), 0);
        let _ = p.poll(0);
        let mut t: u64 = 1;
        p.enqueue(fec_keyframe(1, 6, 2, 4_096, t), t);
        let mut seq = 2u32;
        let mut last_arrival = t;
        while t < 2_500 {
            let _ = p.poll(t);
            while last_arrival + 83 <= t {
                last_arrival += 83;
                p.enqueue(frame(seq, false, 1, 2_500, last_arrival), last_arrival);
                seq += 1;
            }
            t += 5;
        }
        let expired = p.stats().expired_frames;
        assert!(
            expired >= min_expired && expired <= max_expired,
            "rate {rate_kbps}: expired {expired}, wanted [{min_expired}, {max_expired}]"
        );
    }
}

proptest! {
    /// Sliding-window rate bound: bytes released in any 1s window
    /// never exceed rate × 1.25 (the bucket-cap burst allowance)
    /// plus one fragment of slack.
    #[test]
    fn release_rate_bounded(
        rate_kbps in 100u32..1200,
        frames in proptest::collection::vec((1usize..5, 512usize..4_096), 1..20),
    ) {
        let mut p = VideoPacer::new(rate_kbps);
        let mut t: u64 = 0;
        let mut events: Vec<(u64, usize)> = Vec::new();
        for (i, (frags, bytes)) in frames.iter().enumerate() {
            let seq = u32::try_from(i).unwrap();
            p.enqueue(frame(seq, i % 5 == 0, *frags, *bytes, t), t);
        }
        for _ in 0..2_000 {
            let released = p.poll(t);
            // The bound is on WIRE bytes — what the bucket meters.
            let released_bytes: usize =
                released.iter().map(|(_, _, e)| envelope_wire_cost(e)).sum();
            if released_bytes > 0 {
                events.push((t, released_bytes));
            }
            if p.stats().queue_depth == 0 {
                break;
            }
            t += p.next_poll_in_ms(t).max(1);
        }
        // One window of refill + the initial burst (bucket cap has
        // a one-fragment-wire floor for low rates) + one fragment
        // of wire slack. Integer math mirrors the pacer's own
        // milli-byte units.
        let bytes_per_sec = u64::from(rate_kbps) * 125;
        let one_fragment_wire =
            (FRAGMENT_PAYLOAD_LIMIT + PER_FRAGMENT_OVERHEAD_BYTES) as u64;
        // div_ceil: the pacer's cap is exact in MILLI-bytes, so a
        // truncating byte division here undercounts by <1 byte.
        let burst_cap = bytes_per_sec.div_ceil(4).max(one_fragment_wire + 4_096);
        let window_budget = bytes_per_sec + burst_cap + one_fragment_wire;
        for (start, _) in &events {
            let in_window: usize = events
                .iter()
                .filter(|(ts, _)| *ts >= *start && *ts < start + 1_000)
                .map(|(_, b)| b)
                .sum();
            prop_assert!(
                (in_window as u64) <= window_budget,
                "window starting {start} released {in_window} > budget {window_budget}"
            );
        }
    }

    /// Saturation never sheds the incoming keyframe while ANY
    /// droppable non-keyframe is queued.
    #[test]
    fn newest_keyframe_survives_saturation(extra in 1usize..20) {
        let mut p = VideoPacer::new(350);
        for seq in 0..(MAX_QUEUED_FRAMES + extra) {
            p.enqueue(frame(u32::try_from(seq).unwrap(), false, 1, 1_000, 0), 0);
        }
        let dropped = p.enqueue(frame(999, true, 1, 1_000, 0), 0);
        prop_assert!(dropped >= 1);
        let stats = p.stats();
        prop_assert_eq!(stats.queue_depth, MAX_QUEUED_FRAMES);
        // The keyframe we just enqueued must be in the queue.
        let has_key = p.queue.iter().any(|f| f.frame_seq == 999);
        prop_assert!(has_key, "incoming keyframe must survive");
    }

    /// Fragment order within a frame is preserved across polls.
    #[test]
    fn fragment_order_preserved(frags in 2usize..8) {
        let mut p = VideoPacer::new(200);
        let mut f = frame(7, true, frags, 5_000, 0);
        // Tag each fragment's frag_index so order is observable.
        for (i, env) in f.envelopes.iter_mut().enumerate() {
            if let CommunityEnvelope::Control(ControlPayload::VideoFragment {
                frag_index, ..
            }) = env
            {
                *frag_index = u8::try_from(i).unwrap();
            }
        }
        p.enqueue(f, 0);
        let mut seen = Vec::new();
        let mut t = 0;
        for _ in 0..1_000 {
            for (_, _, env) in p.poll(t) {
                if let CommunityEnvelope::Control(ControlPayload::VideoFragment {
                    frag_index, ..
                }) = env
                {
                    seen.push(frag_index);
                }
            }
            if seen.len() == frags {
                break;
            }
            t += p.next_poll_in_ms(t).max(1);
        }
        let expected: Vec<u8> = (0..frags).map(|i| u8::try_from(i).unwrap()).collect();
        prop_assert_eq!(seen, expected);
    }

    /// A mid-stream rate change takes effect (higher rate drains
    /// the same queue strictly faster).
    #[test]
    fn rate_change_respected(low in 100u32..200, high in 600u32..1200) {
        let drain_time = |rate_after: u32| -> u64 {
            let mut p = VideoPacer::new(150);
            for seq in 0..10 {
                p.enqueue(frame(seq, true, 2, 20_000, 0), 0);
            }
            let _ = p.poll(0); // initialize bucket
            p.set_rate_kbps(rate_after);
            let mut t: u64 = 0;
            for _ in 0..100_000 {
                if p.stats().queue_depth == 0 {
                    break;
                }
                t += p.next_poll_in_ms(t).max(1);
                let _ = p.poll(t);
            }
            t
        };
        let slow = drain_time(low);
        let fast = drain_time(high);
        prop_assert!(fast < slow, "high rate {high} ({fast}ms) must drain faster than {low} ({slow}ms)");
    }
}
