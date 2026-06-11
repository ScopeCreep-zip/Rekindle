//! Video send pacer — the libwebrtc `PacingController` analog.
//!
//! Encoded frames arrive as bursts of signed fragments (a keyframe can
//! be 4+ × 28 KB envelopes in one call); firing them at the Veilid
//! routes unmetered is what starved voice. The pacer releases
//! fragments through a token bucket at the budgeted video rate,
//! spreading keyframe bursts and shedding stale frames under
//! saturation.
//!
//! Audio-first is STRUCTURAL: voice frames never enter this queue —
//! its input type only carries video envelopes, and the voice
//! transport sends directly. Video can therefore never delay audio at
//! the sender, mirroring how WebRTC audio bypasses the pacer.
//!
//! Pure logic: time is a parameter (`now_ms`), no I/O — the async
//! driver lives in `send_pacer.rs`.

use std::collections::VecDeque;

use rekindle_protocol::dht::community::envelope::{CommunityEnvelope, ControlPayload};

/// Saturation cap on whole frames awaiting release (~2 s at 15 fps).
pub const MAX_QUEUED_FRAMES: usize = 30;

/// Non-keyframes older than this are stale (a fresher frame supersedes
/// them) and are expired un-sent. Keyframes are exempt: at the 4 s
/// keyframe cadence a late keyframe is still the only sync point a
/// joining receiver will get for seconds — it ships late rather than
/// never.
pub const MAX_QUEUE_AGE_MS: u64 = 500;

/// One encoded frame's worth of signed envelopes awaiting release.
#[derive(Debug, Clone)]
pub struct PacedFrame {
    pub community_id: String,
    pub channel_id: String,
    pub stream_id: [u8; 16],
    pub frame_seq: u32,
    pub keyframe: bool,
    /// Signed data + parity fragment envelopes, in send order.
    pub envelopes: Vec<CommunityEnvelope>,
    /// Total media payload bytes across `envelopes` (what the token
    /// bucket meters).
    pub bytes: usize,
    pub enqueued_ms: u64,
}

/// Media payload size of one fragment envelope — the bucket meters
/// media bytes; envelope framing overhead is proportional and small.
fn envelope_media_bytes(envelope: &CommunityEnvelope) -> usize {
    match envelope {
        CommunityEnvelope::Control(
            ControlPayload::VideoFragment { payload, .. }
            | ControlPayload::VideoParityFragment { payload, .. },
        ) => payload.len(),
        _ => 0,
    }
}

/// Running totals for the 5 s driver summary + tests.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PacerStats {
    pub sent_fragments: u64,
    pub dropped_frames: u64,
    pub expired_frames: u64,
    pub queue_depth: usize,
    pub rate_kbps: u32,
}

/// Token-bucket pacer over whole frames, releasing at FRAGMENT
/// granularity so multi-fragment keyframes spread across polls.
pub struct VideoPacer {
    rate_kbps: u32,
    /// Token bucket in MILLI-bytes (bytes × 1000) so the refill math is
    /// exact integers: 1 kbps = 125 milli-bytes per millisecond.
    bucket_millibytes: u64,
    last_refill_ms: u64,
    /// First-refill latch — a sentinel value of `last_refill_ms` would
    /// collide with a legitimate `now_ms == 0` and re-init the bucket
    /// to full on every poll.
    refill_initialized: bool,
    queue: VecDeque<PacedFrame>,
    /// Index of the next envelope to release within the FRONT frame.
    /// `> 0` marks the front frame in-flight — it always completes
    /// (receivers shouldn't get half a frame because policy changed).
    front_cursor: usize,
    sent_fragments: u64,
    dropped_frames: u64,
    expired_frames: u64,
}

impl VideoPacer {
    #[must_use]
    pub fn new(start_kbps: u32) -> Self {
        Self {
            rate_kbps: start_kbps.max(1),
            bucket_millibytes: 0,
            last_refill_ms: 0,
            refill_initialized: false,
            queue: VecDeque::new(),
            front_cursor: 0,
            sent_fragments: 0,
            dropped_frames: 0,
            expired_frames: 0,
        }
    }

    pub fn set_rate_kbps(&mut self, kbps: u32) {
        self.rate_kbps = kbps.max(1);
    }

    /// Milli-bytes per millisecond at the current rate: kbps × 1000
    /// bits/s ÷ 8 bytes ÷ 1000 ms × 1000 milli = kbps × 125. Exact.
    fn millibytes_per_ms(&self) -> u64 {
        u64::from(self.rate_kbps) * 125
    }

    /// Bucket capacity (milli-bytes): a quarter-second of budget, but
    /// always at least one max-size fragment so progress is possible
    /// at any rate.
    fn bucket_cap(&self) -> u64 {
        let quarter_second = self.millibytes_per_ms() * 250;
        let one_fragment = (crate::fragment::FRAGMENT_PAYLOAD_LIMIT as u64 + 4096) * 1000;
        quarter_second.max(one_fragment)
    }

    fn refill(&mut self, now_ms: u64) {
        if !self.refill_initialized {
            self.refill_initialized = true;
            self.last_refill_ms = now_ms;
            self.bucket_millibytes = self.bucket_cap();
            return;
        }
        let dt_ms = now_ms.saturating_sub(self.last_refill_ms);
        self.last_refill_ms = now_ms;
        let added = self.millibytes_per_ms().saturating_mul(dt_ms);
        self.bucket_millibytes = self
            .bucket_millibytes
            .saturating_add(added)
            .min(self.bucket_cap());
    }

    /// Enqueue a frame, shedding under saturation. Returns the number
    /// of frames dropped to make room. Shed order: oldest droppable
    /// NON-keyframe first, then oldest droppable keyframe — and the
    /// incoming keyframe itself is never the victim while any
    /// droppable frame exists (it's the freshest sync point). The
    /// in-flight front frame is never dropped.
    pub fn enqueue(&mut self, frame: PacedFrame, now_ms: u64) -> usize {
        self.expire_stale(now_ms);
        let mut dropped = 0;
        while self.queue.len() >= MAX_QUEUED_FRAMES {
            let first_droppable = usize::from(self.front_cursor > 0);
            let victim = self
                .queue
                .iter()
                .enumerate()
                .skip(first_droppable)
                .find(|(_, f)| !f.keyframe)
                .map(|(i, _)| i)
                .or_else(|| {
                    self.queue
                        .iter()
                        .enumerate()
                        .skip(first_droppable)
                        .find(|(_, f)| f.keyframe)
                        .map(|(i, _)| i)
                });
            if let Some(i) = victim {
                self.queue.remove(i);
                self.dropped_frames += 1;
                dropped += 1;
            } else {
                // Only the in-flight frame remains (pathological
                // MAX=1-style config) — refuse the incoming frame.
                self.dropped_frames += 1;
                return dropped + 1;
            }
        }
        self.queue.push_back(frame);
        dropped
    }

    fn expire_stale(&mut self, now_ms: u64) {
        let first_droppable = usize::from(self.front_cursor > 0);
        let mut i = first_droppable;
        while i < self.queue.len() {
            let f = &self.queue[i];
            if !f.keyframe && now_ms.saturating_sub(f.enqueued_ms) > MAX_QUEUE_AGE_MS {
                self.queue.remove(i);
                self.expired_frames += 1;
            } else {
                i += 1;
            }
        }
    }

    /// Refill the bucket, expire stale frames, and release every
    /// fragment the budget affords (FIFO, fragment granularity).
    pub fn poll(&mut self, now_ms: u64) -> Vec<(String, String, CommunityEnvelope)> {
        self.refill(now_ms);
        self.expire_stale(now_ms);
        let mut out = Vec::new();
        loop {
            let Some(front) = self.queue.front() else {
                break;
            };
            let Some(envelope) = front.envelopes.get(self.front_cursor) else {
                // Front frame fully released.
                self.queue.pop_front();
                self.front_cursor = 0;
                continue;
            };
            let cost = envelope_media_bytes(envelope) as u64 * 1000;
            if cost > self.bucket_millibytes {
                break;
            }
            self.bucket_millibytes -= cost;
            let front = self.queue.front().expect("checked above");
            out.push((
                front.community_id.clone(),
                front.channel_id.clone(),
                front.envelopes[self.front_cursor].clone(),
            ));
            self.front_cursor += 1;
            self.sent_fragments += 1;
        }
        out
    }

    /// Milliseconds until the next fragment becomes affordable: 0 when
    /// one is already due, a refill estimate (≥ 5 ms) when the bucket
    /// is short, and a long idle tick when the queue is empty (the
    /// driver also wakes on enqueue).
    #[must_use]
    pub fn next_poll_in_ms(&self, _now_ms: u64) -> u64 {
        let Some(front) = self.queue.front() else {
            return 1_000;
        };
        let Some(envelope) = front.envelopes.get(self.front_cursor) else {
            return 0;
        };
        let cost = envelope_media_bytes(envelope) as u64 * 1000;
        if cost <= self.bucket_millibytes {
            return 0;
        }
        let deficit = cost - self.bucket_millibytes;
        let per_ms = self.millibytes_per_ms().max(1);
        deficit.div_ceil(per_ms).max(5)
    }

    #[must_use]
    pub fn stats(&self) -> PacerStats {
        PacerStats {
            sent_fragments: self.sent_fragments,
            dropped_frames: self.dropped_frames,
            expired_frames: self.expired_frames,
            queue_depth: self.queue.len(),
            rate_kbps: self.rate_kbps,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
        // 350 kbps = 43_750 B/s. A 3×10 KB frame: initial bucket
        // (capped) affords some, the rest trickles.
        let mut p = VideoPacer::new(350);
        p.enqueue(frame(1, true, 3, 10_000, 0), 0);
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
        p.enqueue(frame(1, false, 4, 10_000, 0), 0);
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
                .filter(|(_, _, e)| envelope_media_bytes(e) == 10_000)
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
        p.enqueue(frame(1, true, 1, 10_000, 0), 0);
        p.enqueue(frame(2, false, 1, 10_000, 0), 0);
        p.enqueue(frame(3, false, 1, 10_000, 0), 0);
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

    proptest! {
        /// Sliding-window rate bound: bytes released in any 1s window
        /// never exceed rate × 1.25 (the bucket-cap burst allowance)
        /// plus one fragment of slack.
        #[test]
        fn release_rate_bounded(
            rate_kbps in 100u32..1200,
            frames in proptest::collection::vec((1usize..5, 1_000usize..28_000), 1..20),
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
                let released_bytes: usize =
                    released.iter().map(|(_, _, e)| envelope_media_bytes(e)).sum();
                if released_bytes > 0 {
                    events.push((t, released_bytes));
                }
                if p.stats().queue_depth == 0 {
                    break;
                }
                t += p.next_poll_in_ms(t).max(1);
            }
            // One window of refill + the initial burst (bucket cap has
            // a one-fragment floor for low rates) + one fragment slack.
            // Integer math mirrors the pacer's own milli-byte units.
            let bytes_per_sec = u64::from(rate_kbps) * 125;
            // div_ceil: the pacer's cap is exact in MILLI-bytes, so a
            // truncating byte division here undercounts by <1 byte.
            let burst_cap = bytes_per_sec.div_ceil(4).max(32_864);
            let window_budget = bytes_per_sec + burst_cap + 28_000;
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
}
