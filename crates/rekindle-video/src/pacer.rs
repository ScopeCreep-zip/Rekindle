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

/// Per-fragment wire overhead beyond the media payload: ~250 B of our
/// own envelope (VideoFragment header + Ed25519 signature + packed
/// Cap'n Proto framing) plus ~1.1 KiB of first-hop Veilid chrome
/// (ENV0 header/signature + ~6 onion layers + RoutedOperation). At
/// the 4 KiB fragment budget this is ~25% of wire bytes — far too big
/// to ignore; the bucket must meter what the network actually carries
/// (libwebrtc's pacing `include overhead` model).
pub const PER_FRAGMENT_OVERHEAD_BYTES: usize = 1_400;

/// Wire cost of one fragment envelope: payload + per-fragment chrome.
fn envelope_wire_cost(envelope: &CommunityEnvelope) -> usize {
    match envelope {
        CommunityEnvelope::Control(
            ControlPayload::VideoFragment { payload, .. }
            | ControlPayload::VideoParityFragment { payload, .. },
        ) => payload.len() + PER_FRAGMENT_OVERHEAD_BYTES,
        _ => 0,
    }
}

/// Receiver-countable payload of one envelope: DATA fragments only.
/// Parity is consumed inside the reassembler and never reaches the
/// receiver's goodput accounting — it belongs in the wire denominator
/// of the payload share but not the numerator.
fn envelope_data_payload_bytes(envelope: &CommunityEnvelope) -> usize {
    match envelope {
        CommunityEnvelope::Control(ControlPayload::VideoFragment { payload, .. }) => payload.len(),
        _ => 0,
    }
}

/// Running totals for the 5 s driver summary + tests.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PacerStats {
    pub sent_fragments: u64,
    pub dropped_frames: u64,
    pub expired_frames: u64,
    /// Keyframes whose wire cost exceeded a full TTL of budget at
    /// intake — no pacing policy can deliver such a frame without
    /// starving every delta behind it (the R4 "forbidden band").
    pub oversized_keyframes: u64,
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
    oversized_keyframes: u64,
    /// Released DATA-fragment payload bytes (the receiver-countable
    /// share numerator) since startup.
    released_data_payload_bytes: u64,
    /// Released wire bytes — payload + overhead, data + parity (the
    /// share denominator) since startup.
    released_wire_bytes: u64,
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
            oversized_keyframes: 0,
            released_data_payload_bytes: 0,
            released_wire_bytes: 0,
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
    /// always at least one max-size fragment AT WIRE COST so progress
    /// is possible at any rate.
    fn bucket_cap(&self) -> u64 {
        let quarter_second = self.millibytes_per_ms() * 250;
        let one_fragment = (crate::fragment::FRAGMENT_PAYLOAD_LIMIT as u64
            + PER_FRAGMENT_OVERHEAD_BYTES as u64
            + 4096)
            * 1000;
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
        // R4 intake guard: a keyframe whose wire cost exceeds a full
        // delta-TTL of budget cannot be delivered without expiring
        // every delta queued behind it — count it so the driver can
        // surface the encoder/budget mismatch (the frame still ships;
        // dropping the only sync point would be worse).
        if frame.keyframe {
            let wire: u64 = frame
                .envelopes
                .iter()
                .map(|e| envelope_wire_cost(e) as u64)
                .sum();
            // milli-bytes/ms × TTL ms ÷ 1000 = whole bytes over one TTL.
            let ttl_budget_bytes = self.millibytes_per_ms() * MAX_QUEUE_AGE_MS / 1000;
            if wire > ttl_budget_bytes {
                self.oversized_keyframes += 1;
            }
        }
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
            let wire = envelope_wire_cost(envelope);
            let cost = wire as u64 * 1000;
            if cost > self.bucket_millibytes {
                break;
            }
            self.bucket_millibytes -= cost;
            self.released_wire_bytes += wire as u64;
            self.released_data_payload_bytes += envelope_data_payload_bytes(envelope) as u64;
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

    /// Q10 fixed-point payload share of released traffic:
    /// receiver-countable data payload ÷ wire bytes, clamped to
    /// [0.25, 1.0]. `None` until at least one full fragment has been
    /// released. Feeds the wire↔media unit conversions in `budget` —
    /// the AIMD runs in wire units, the encoder in media units, and
    /// this measured ratio is the bridge (libwebrtc's
    /// `WithOverhead` model).
    #[must_use]
    pub fn payload_share_q10(&self) -> Option<u32> {
        if self.released_wire_bytes < (crate::fragment::FRAGMENT_PAYLOAD_LIMIT as u64) {
            return None;
        }
        let q10 = self.released_data_payload_bytes * 1024 / self.released_wire_bytes;
        Some(u32::try_from(q10.clamp(256, 1024)).expect("clamped to <= 1024"))
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
        let cost = envelope_wire_cost(envelope) as u64 * 1000;
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
            oversized_keyframes: self.oversized_keyframes,
            queue_depth: self.queue.len(),
            rate_kbps: self.rate_kbps,
        }
    }
}

#[cfg(test)]
#[path = "pacer/tests.rs"]
mod tests;
