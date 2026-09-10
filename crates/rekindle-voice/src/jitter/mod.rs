use crate::transport::VoicePacket;
use rekindle_media_stats::{ReceptionMetrics, ReceptionTracker};
use std::collections::BTreeMap;

/// Lower bound on the adaptive playout target — below this the buffer
/// can't absorb even one frame of reorder.
const JITTER_MIN_MS: u32 = 40;
/// Upper bound on the adaptive playout target. Capped at the largest
/// preset (120) so worst-case mouth-to-ear stays inside the 250 ms
/// budget enforced by `tests/latency_budget.rs`.
const JITTER_MAX_MS: u32 = 120;
/// Headroom multiplier over the smoothed jitter estimate, as a
/// rational (5/2 = 2.5× jitter), integer-friendly.
const JITTER_K_NUM: u32 = 5;
const JITTER_K_DEN: u32 = 2;
/// One Opus frame's worth of playout (20 ms) — the slew step and the
/// initial-fill / gap-jump window granularity.
const FRAME_MS: u32 = 20;
/// Consecutive clean 5 s windows required before the target may shrink
/// (prevents collapsing depth on a single quiet window).
const CLEAN_WINDOWS_TO_SHRINK: u32 = 2;
/// Upper bound on the tracked reorder span (packets ≈ ×20 ms). Caps how
/// long the gap jump will wait for a reordered packet before treating
/// the position as lost — 15 ≈ 300 ms, past which a stall is a worse
/// experience than concealing the gap, and it stops one pathologically
/// stale straggler from pinning the window open.
const REORDER_SPAN_MAX_PACKETS: u32 = 15;
/// Reorder distances above this are treated as sequence wrap / a peer
/// resetting its counter, not real reordering — mirrors the reception
/// tracker's own guard so the two never disagree about what a gap is.
const REORDER_IMPLAUSIBLE: u32 = 1000;

/// Adaptive jitter buffer for smoothing out network timing variations.
///
/// Buffers incoming voice packets and releases them at a steady rate
/// to compensate for variable network latency. Includes an initial
/// buffering phase that waits until enough packets have accumulated
/// before allowing playback to start.
///
/// NetEq-style adaptation: `target_delay_ms` tracks the smoothed
/// inter-arrival jitter (RFC 3550) — starting at the config base and
/// growing fast / shrinking slow so it protects audio under jitter
/// without oscillating the initial-fill and gap-jump windows (both
/// derived live from `target_delay_ms`).
pub struct JitterBuffer {
    /// Buffered packets indexed by sequence number.
    buffer: BTreeMap<u32, VoicePacket>,
    /// Current ADAPTIVE buffer depth in milliseconds. Mutated only via
    /// [`Self::set_target_delay_ms`] so the derived windows stay in sync.
    target_delay_ms: u32,
    /// Config-preset floor — the adaptive target starts here and never
    /// shrinks below it.
    base_delay_ms: u32,
    /// The reception model for this peer: RFC 3550 interarrival jitter
    /// (which drives the adaptive target below) plus the RFC 3611
    /// loss/discard/burst-gap split (which the receiver report sends
    /// back to the sender). The buffer is the only component that sees
    /// every arrival with its sequence and sender timestamp, so it is
    /// where the measurement belongs — and keeping one tracker is what
    /// stops the playout estimate and the reported estimate drifting.
    reception: ReceptionTracker,
    /// Consecutive 5 s windows with zero late drops — gates the slow
    /// shrink in [`Self::recompute_target`].
    clean_windows: u32,
    /// Next expected sequence number for playback.
    next_playback_seq: u32,
    /// Maximum number of packets to buffer before dropping old ones.
    max_packets: usize,
    /// Whether the initial buffering phase is complete.
    initial_fill_done: bool,
    /// Arrival time of the first packet, in the caller's monotonic
    /// millisecond clock (for initial-fill timing). Resets to `None`
    /// with the buffer so the fill origin tracks each fresh start.
    first_arrival_ms: Option<u64>,
    /// Packets discarded by the over-capacity trim since the last
    /// `take_drops` — the receive-side health signal the quality event
    /// surfaces (Phase 5).
    overflow_drops: u64,
    /// Packets discarded for arriving after their playback slot.
    late_drops: u64,
    /// Consecutive `pop()` misses at the current playback position —
    /// drives the gap jump in [`Self::note_miss_and_maybe_jump`].
    gap_misses: u32,
    /// Highest sequence number seen so far — the reference the reorder
    /// span is measured against.
    highest_seq_seen: Option<u32>,
    /// Largest recent out-of-order distance, in packets: how far behind
    /// the highest-seen sequence a packet has arrived. The media route
    /// runs `Sequencing::PreferUnordered` (UDP-class, low latency but
    /// reordering), and absorbing that reordering is the jitter buffer's
    /// job. This sizes the gap-jump window so the jump waits for a
    /// reordered-but-not-lost packet instead of skipping past it and
    /// discarding it on arrival — the receiver-side cause of the live
    /// 86 % discard (proven in `reordered_delivery_reproduces_...`).
    /// Bounded and decayed so one stale straggler can't pin it.
    reorder_span: u32,
}

impl JitterBuffer {
    /// Create a new jitter buffer whose adaptive target starts at the
    /// given base delay (the config preset). Adaptation grows it toward
    /// measured jitter and shrinks it back toward this base.
    pub fn new(target_delay_ms: u32) -> Self {
        Self {
            buffer: BTreeMap::new(),
            target_delay_ms,
            base_delay_ms: target_delay_ms,
            reception: ReceptionTracker::new(FRAME_MS),
            clean_windows: 0,
            next_playback_seq: 0,
            max_packets: 50,
            initial_fill_done: false,
            first_arrival_ms: None,
            overflow_drops: 0,
            late_drops: 0,
            gap_misses: 0,
            highest_seq_seen: None,
            reorder_span: 0,
        }
    }

    /// Push an incoming packet into the buffer.
    ///
    /// `arrival_ms` is the caller's monotonic millisecond clock at the
    /// moment the packet arrived (the receive loop's `local_ms()`). The
    /// buffer keeps no `Instant` of its own, so its timing behavior is
    /// fully determined by its inputs and unit-tests deterministically.
    pub fn push(&mut self, packet: VoicePacket, arrival_ms: u64) {
        let seq = packet.sequence;

        // Record the first arrival as the initial-fill origin — it
        // resets with the buffer. The tracker itself only ever uses the
        // delta between consecutive transits, so any fixed clock origin
        // works: a constant offset between the two machines cancels out.
        self.first_arrival_ms.get_or_insert(arrival_ms);

        // Track how far out of order the route is delivering. When a
        // packet lands behind the highest sequence seen, that distance
        // is how deep the gap jump must wait before it may treat the
        // position as lost — otherwise it skips reordered-but-in-flight
        // packets and discards them on arrival. Learned continuously so
        // the window is already sized by the time a gap opens.
        match self.highest_seq_seen {
            Some(hi) if seq > hi => self.highest_seq_seen = Some(seq),
            Some(hi) if seq < hi => {
                let dist = hi - seq;
                if dist <= REORDER_IMPLAUSIBLE {
                    self.reorder_span = self.reorder_span.max(dist).min(REORDER_SPAN_MAX_PACKETS);
                }
            }
            None => self.highest_seq_seen = Some(seq),
            _ => {}
        }

        // A packet past its playback slot arrived but cannot be played.
        // RFC 3611 calls that a *discard*, not a loss — the distinction
        // matters because a discard means our buffer is too shallow
        // while a loss means the network dropped it, and the two have
        // opposite fixes.
        let late = self.initial_fill_done && seq < self.next_playback_seq;
        self.observe_arrival(seq, packet.timestamp, arrival_ms, !late);

        if late {
            self.late_drops += 1;
            tracing::trace!(
                seq,
                expected = self.next_playback_seq,
                "dropping late packet"
            );
            return;
        }

        self.buffer.insert(seq, packet);

        // Trim if buffer is too large
        while self.buffer.len() > self.max_packets {
            self.buffer.pop_first();
            self.overflow_drops += 1;
            // Overflow is the other discard class: it arrived in time
            // and we threw it away for want of room.
            self.reception.note_external_discards(1);
            if self.initial_fill_done {
                self.next_playback_seq += 1;
            }
        }
    }

    /// The reception model for this peer — RFC 3550 jitter plus the
    /// RFC 3611 loss/discard/burst-gap split. This is what the receiver
    /// report sends back to the sender, and it is the same estimate the
    /// adaptive target is derived from.
    pub fn reception_metrics(&self) -> ReceptionMetrics {
        self.reception.metrics()
    }

    /// Drain the drop counters (overflow, late) accumulated since the
    /// last call — read on the 5 s quality cadence.
    pub fn take_drops(&mut self) -> (u64, u64) {
        let drops = (self.overflow_drops, self.late_drops);
        self.overflow_drops = 0;
        self.late_drops = 0;
        drops
    }

    /// Fold one arrival into the reception model, then re-derive the
    /// adaptive target from it. Pure in its inputs (no `Instant`), so a
    /// whole arrival pattern unit-tests deterministically; [`Self::push`]
    /// supplies the real clock.
    fn observe_arrival(&mut self, sequence: u32, sender_ms: u64, arrival_ms: u64, usable: bool) {
        self.reception
            .on_packet(sequence, sender_ms, arrival_ms, usable);
        // An interarrival estimate needs two arrivals to exist. Until
        // then there is nothing to adapt to, and recomputing would
        // apply the MIN clamp to a base the config set deliberately
        // lower — `base_from_config_not_hardcoded` pins that.
        if self.reception.metrics().packets_received > 1 {
            self.recompute_target();
        }
    }

    /// Re-derive the adaptive target from the smoothed jitter:
    /// `target = clamp(base + 2.5·jitter, max(MIN, base), MAX)` with an
    /// asymmetric slew — grow immediately (protect audio), shrink one
    /// 20 ms step at a time and only after `CLEAN_WINDOWS_TO_SHRINK`
    /// clean windows. Mutates only via `set_target_delay_ms` so the
    /// initial-fill / gap-jump windows track it.
    fn recompute_target(&mut self) {
        let jitter_ms = self.reception.metrics().jitter_ms;
        let headroom = jitter_ms.saturating_mul(JITTER_K_NUM) / JITTER_K_DEN;
        let want = self.base_delay_ms.saturating_add(headroom);
        let clamped = want.clamp(JITTER_MIN_MS.max(self.base_delay_ms), JITTER_MAX_MS);
        let next = if clamped > self.target_delay_ms {
            clamped
        } else if self.clean_windows >= CLEAN_WINDOWS_TO_SHRINK {
            self.target_delay_ms.saturating_sub(FRAME_MS).max(clamped)
        } else {
            self.target_delay_ms
        };
        self.set_target_delay_ms(next);
    }

    /// Called on the 5 s quality cadence with this window's late-drop
    /// count (already drained by `take_drops`). Late drops mean the
    /// target is too low — grow one step now and reset the clean
    /// counter; a clean window advances it toward the shrink gate.
    pub fn note_window_health(&mut self, late_drops: u64) {
        if late_drops > 0 {
            self.clean_windows = 0;
            let bumped = (self.target_delay_ms + FRAME_MS).min(JITTER_MAX_MS);
            self.set_target_delay_ms(bumped);
        } else {
            self.clean_windows = self.clean_windows.saturating_add(1);
            // Let the reorder window relax one packet per clean 5 s
            // window: reordering is a route property that outlives any
            // single burst, so decay slowly rather than dropping the
            // guard the moment one quiet window passes.
            self.reorder_span = self.reorder_span.saturating_sub(1);
        }
        self.recompute_target();
    }

    /// Pop the next packet for playback, if available.
    ///
    /// Returns `None` if the initial fill phase hasn't completed yet
    /// or the next expected packet hasn't arrived (packet loss / buffering).
    ///
    /// `now_ms` is the caller's monotonic millisecond clock at this
    /// playout tick — the same clock passed to [`Self::push`] — used by
    /// the initial-fill time window.
    pub fn pop(&mut self, now_ms: u64) -> Option<VoicePacket> {
        // Don't start playback until initial fill is complete
        if !self.initial_fill_done {
            if !self.check_initial_fill(now_ms) {
                return None;
            }
            // Set next_playback_seq to the first available sequence
            if let Some(&first_seq) = self.buffer.keys().next() {
                self.next_playback_seq = first_seq;
            }
        }

        let packet = self.buffer.remove(&self.next_playback_seq);
        if packet.is_some() {
            self.next_playback_seq += 1;
            self.gap_misses = 0;
        }
        packet
    }

    /// One playout tick failed to find the expected packet. After a
    /// full jitter window of consecutive misses (the packet is lost,
    /// not late), jump the gap: resume from the oldest buffered packet
    /// — the voice analog of the video playout buffer's keyframe jump.
    /// Without this a single lost packet stalls the stream forever:
    /// playback stays pinned at the missing seq while the overflow
    /// trim discards one GOOD packet per arrival (observed live as
    /// rx_overflow_drops in the hundreds per 5 s with dead audio until
    /// the participant timed out and re-seeded).
    pub fn note_miss_and_maybe_jump(&mut self) -> Option<VoicePacket> {
        if !self.initial_fill_done {
            return None;
        }
        self.gap_misses += 1;
        // Wait at least the adaptive jitter window AND the observed
        // reorder span before giving up on the missing position: under
        // `PreferUnordered` the "missing" packet is usually reordered,
        // not lost, and arrives within `reorder_span` more packets. The
        // `+1` clears the span itself. This extends only the gap-
        // recovery wait, never the steady-state playout depth, so it
        // does not tax the mouth-to-ear budget.
        // `+2`, not `+1`: under sustained reordering the player runs a
        // full span behind, and `gap_misses` carries across the boundary
        // where one reordered block hands off to the next, so a bare
        // `span + 1` fires exactly at that seam. One extra tick of margin
        // clears it, at the cost of a single 20 ms tick more concealment
        // before a genuinely lost position is skipped.
        let window_ticks = (self.target_delay_ms / FRAME_MS)
            .max(self.reorder_span + 2)
            .max(1);
        if self.gap_misses < window_ticks || self.buffer.is_empty() {
            return None;
        }
        let (&seq, _) = self.buffer.iter().next()?;
        self.gap_misses = 0;
        self.next_playback_seq = seq.wrapping_add(1);
        self.buffer.remove(&seq)
    }

    /// FEC reconstructed the frame at the current playback position —
    /// advance past it so the next pop returns the real buffered packet
    /// the FEC data was peeked from, instead of re-concealing the same
    /// position every tick.
    pub fn advance_after_fec(&mut self) {
        self.next_playback_seq = self.next_playback_seq.wrapping_add(1);
        self.gap_misses = 0;
    }

    /// Check if the initial fill phase should complete.
    ///
    /// Completes when either:
    /// - Enough time has elapsed since first packet (`target_delay_ms`)
    /// - Enough packets have accumulated (`target_delay_ms` / 20ms)
    fn check_initial_fill(&mut self, now_ms: u64) -> bool {
        let target_packets = (self.target_delay_ms / 20).max(1) as usize;

        if self.buffer.len() >= target_packets {
            self.initial_fill_done = true;
            return true;
        }

        if let Some(first_arrival_ms) = self.first_arrival_ms {
            if now_ms.saturating_sub(first_arrival_ms) >= u64::from(self.target_delay_ms) {
                self.initial_fill_done = true;
                return true;
            }
        }

        false
    }

    /// Peek at the next buffered packet after the expected one (for FEC recovery).
    ///
    /// When `pop()` returns `None` (current packet missing), this peeks at
    /// `next_playback_seq + 1` to check if FEC recovery is possible.
    /// Returns the audio data of the next packet if available.
    ///
    /// Gated by the reorder span: FEC recovery **advances past** the
    /// missing position (`advance_after_fec`), so firing it while the
    /// position could still be filled by a reordered-in-flight packet
    /// would skip that packet and discard it on arrival — the same
    /// over-advance the gap jump guards against, via a different door.
    /// So hold FEC until `gap_misses` has cleared the observed reorder
    /// span. With no reordering (`reorder_span == 0`) this is `> 0`, so
    /// FEC still fires on the first miss — the fast single-loss path is
    /// unchanged; only reordering makes it wait.
    pub fn peek_next_audio_data(&self) -> Option<&[u8]> {
        if !self.initial_fill_done || self.gap_misses <= self.reorder_span {
            return None;
        }
        let next_seq = self.next_playback_seq.wrapping_add(1);
        self.buffer.get(&next_seq).map(|p| p.audio_data.as_slice())
    }

    /// Get the current buffer depth (number of buffered packets).
    pub fn depth(&self) -> usize {
        self.buffer.len()
    }

    /// Get the target delay in milliseconds.
    pub fn target_delay_ms(&self) -> u32 {
        self.target_delay_ms
    }

    /// Set a new target delay.
    pub fn set_target_delay_ms(&mut self, ms: u32) {
        self.target_delay_ms = ms;
    }

    /// Reset the buffer (e.g., on reconnect). Returns the adaptive
    /// target to the config base and clears the jitter estimator.
    pub fn reset(&mut self) {
        self.buffer.clear();
        self.next_playback_seq = 0;
        self.initial_fill_done = false;
        self.first_arrival_ms = None;
        self.gap_misses = 0;
        self.target_delay_ms = self.base_delay_ms;
        self.reception = ReceptionTracker::new(FRAME_MS);
        self.clean_windows = 0;
        self.highest_seq_seen = None;
        self.reorder_span = 0;
    }
}

#[cfg(test)]
mod tests;
