//! Receiver-side transport loss + goodput window for ONE video sender.
//!
//! The libwebrtc transport-cc analog. Wire loss is measured over the
//! **transport sequence** the sender's [`VideoPacer`] stamps at egress
//! — gap-free over fragments ACTUALLY transmitted — NEVER over the
//! encoder's `frame_seq`. That distinction is the whole point:
//!
//! A `frame_seq`-gap loss model (the retired frontend `playout_buffer`
//! accounting) counted every skipped `frame_seq` as a lost packet. But
//! the pacer legitimately EXPIRES stale frames it can't send in time,
//! and those frames were assigned a `frame_seq` at encode. The receiver
//! then read sender-side pacing drops as ~85 % network loss, the
//! sender's AIMD ([`crate::budget`]) collapsed to its floor, the lower
//! rate expired even more frames, and the stream pinned at the floor —
//! a phantom-loss death spiral (observed live: 350 → 100 kbps in ~10 s,
//! stuck for the rest of the call). Frames the pacer expires never get
//! a `transport_seq`, so counting over it can't manufacture that loss.
//!
//! Pure and transport-agnostic: fed one [`VideoReceptionWindow::observe`]
//! per authentic received fragment (data OR parity — both are paced and
//! both count toward wire conditions), it emits a [`FrameAckOut`] once
//! [`ACK_WINDOW_MS`] has elapsed. The community and DM receive paths
//! share this one estimator rather than each carrying its own.
//!
//! [`VideoPacer`]: crate::VideoPacer

/// Receiver → sender feedback cadence (the AIMD's input rate). Matches
/// the former frontend `ACK_INTERVAL_MS`. SCReAM's `rate_fb` formula
/// lands ~150–200 ms while a link is constrained; 1 s is the
/// steady-state cadence and keeps the RTCP-style control overhead well
/// under the RFC 3550 5 %-of-session bound.
pub const ACK_WINDOW_MS: u32 = 1_000;

/// A transport-sequence jump larger than this (either direction) inside
/// one window is a pacer restart or a u32 rollover, not real loss —
/// resync the window instead of inventing thousands of lost packets.
/// One window holds only a few dozen fragments (≤ ~30 fps × 1 s × a few
/// fragments), so a legitimate step is never this large.
const SEQ_RESYNC_THRESHOLD: u32 = 1_000;

/// Windowed feedback handed back to a sender so its AIMD
/// ([`crate::budget::target_from_feedback`]) can adapt the encoder +
/// pacer rate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameAckOut {
    pub channel_id: String,
    pub stream_id: [u8; 16],
    /// Highest `frame_seq` observed — informational for the sender
    /// (the AIMD keys on channel, not frame). Mirrors the old ack's
    /// `lastFrameSeq`.
    pub last_frame_seq: u32,
    /// Measured receive goodput this window, kbps.
    pub kbps: u32,
    /// Wire loss over the transport sequence this window, Q8
    /// (`0..=255`, `loss × 256`).
    pub loss_q8: u8,
}

/// Per-sender windowed loss + goodput estimator over the transport
/// sequence. One per `(community, sender)` (or `(peer, stream)` for
/// DM) — see the module docs.
#[derive(Debug, Clone)]
pub struct VideoReceptionWindow {
    /// Lowest transport_seq seen this window (reorder-tolerant min).
    low: Option<u32>,
    /// Highest transport_seq seen this window (reorder-tolerant max).
    high: Option<u32>,
    received: u32,
    bytes: u64,
    window_start_ms: u32,
    // Sticky routing/identity carried into the emitted ack.
    last_frame_seq: u32,
    stream_id: [u8; 16],
    channel_id: String,
}

impl VideoReceptionWindow {
    #[must_use]
    pub fn new(now_ms: u32) -> Self {
        Self {
            low: None,
            high: None,
            received: 0,
            bytes: 0,
            window_start_ms: now_ms,
            last_frame_seq: 0,
            stream_id: [0u8; 16],
            channel_id: String::new(),
        }
    }

    /// Record one authentic received fragment. Returns a windowed
    /// [`FrameAckOut`] when [`ACK_WINDOW_MS`] has elapsed since the
    /// window opened, resetting the window; `None` otherwise.
    pub fn observe(
        &mut self,
        transport_seq: u32,
        frame_seq: u32,
        stream_id: [u8; 16],
        channel_id: &str,
        bytes: usize,
        now_ms: u32,
    ) -> Option<FrameAckOut> {
        // Pacer restart / rollover: a wild jump is not loss.
        if let Some(high) = self.high {
            if transport_seq.abs_diff(high) > SEQ_RESYNC_THRESHOLD {
                self.low = None;
                self.high = None;
                self.received = 0;
                self.bytes = 0;
                self.window_start_ms = now_ms;
            }
        }
        self.low = Some(self.low.map_or(transport_seq, |l| l.min(transport_seq)));
        self.high = Some(self.high.map_or(transport_seq, |h| h.max(transport_seq)));
        self.received = self.received.saturating_add(1);
        self.bytes = self.bytes.saturating_add(bytes as u64);
        self.last_frame_seq = self.last_frame_seq.max(frame_seq);
        self.stream_id = stream_id;
        self.channel_id = channel_id.to_string();

        let elapsed = now_ms.wrapping_sub(self.window_start_ms);
        if elapsed < ACK_WINDOW_MS {
            return None;
        }
        Some(self.close(elapsed, now_ms))
    }

    fn close(&mut self, elapsed_ms: u32, now_ms: u32) -> FrameAckOut {
        let expected = match (self.low, self.high) {
            (Some(l), Some(h)) => h.saturating_sub(l).saturating_add(1),
            _ => 0,
        };
        let lost = expected.saturating_sub(self.received);
        let loss_q8 = if expected == 0 {
            0
        } else {
            u8::try_from((u64::from(lost) * 256 / u64::from(expected)).min(255)).unwrap_or(255)
        };
        // bits ÷ ms = kbit/s. `.max(1)` on ms guards a zero window; the
        // reported rate floors at 1 so a live stream never reads 0.
        let kbps = u32::try_from(
            self.bytes.saturating_mul(8) / u64::from(elapsed_ms.max(1)),
        )
        .unwrap_or(u32::MAX)
        .max(1);
        let out = FrameAckOut {
            channel_id: self.channel_id.clone(),
            stream_id: self.stream_id,
            last_frame_seq: self.last_frame_seq,
            kbps,
            loss_q8,
        };
        // Fresh window; the next fragment re-establishes low/high. Full
        // reset mirrors the retired frontend `takeStats`; the single
        // unmeasured inter-window delta is negligible.
        self.low = None;
        self.high = None;
        self.received = 0;
        self.bytes = 0;
        self.window_start_ms = now_ms;
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SID: [u8; 16] = [0xAB; 16];

    /// Drive `count` contiguous transport sequences starting at `base`,
    /// one per `step_ms`, with `frame_seq` following `frame_seqs` (so a
    /// test can decouple the wire sequence from the encode sequence).
    fn feed(
        w: &mut VideoReceptionWindow,
        base: u32,
        frame_seqs: &[u32],
        bytes: usize,
        start_ms: u32,
        step_ms: u32,
    ) -> Option<FrameAckOut> {
        let mut out = None;
        for (i, fs) in frame_seqs.iter().enumerate() {
            let seq = base + u32::try_from(i).unwrap();
            let now = start_ms + u32::try_from(i).unwrap() * step_ms;
            if let Some(ack) = w.observe(seq, *fs, SID, "ch1", bytes, now) {
                out = Some(ack);
            }
        }
        out
    }

    #[test]
    fn holds_ack_until_window_elapses() {
        let mut w = VideoReceptionWindow::new(0);
        // Three fragments inside the first 100 ms — no ack yet.
        assert!(w.observe(0, 0, SID, "ch1", 1000, 10).is_none());
        assert!(w.observe(1, 1, SID, "ch1", 1000, 20).is_none());
        assert!(w.observe(2, 2, SID, "ch1", 1000, 30).is_none());
        // One past the window boundary emits.
        let ack = w.observe(3, 3, SID, "ch1", 1000, ACK_WINDOW_MS + 1).unwrap();
        assert_eq!(ack.loss_q8, 0, "contiguous transport seq = no loss");
        assert_eq!(ack.last_frame_seq, 3);
    }

    #[test]
    fn contiguous_transport_seq_reports_zero_loss() {
        let mut w = VideoReceptionWindow::new(0);
        let seqs: Vec<u32> = (0..40).collect();
        let ack = feed(&mut w, 0, &seqs, 500, 0, 30).expect("window elapses over 40×30ms");
        assert_eq!(ack.loss_q8, 0);
    }

    /// THE regression this whole change exists for: the encoder's
    /// `frame_seq` has big holes (the sender's pacer expired frames),
    /// but the transport sequence is gap-free. Loss MUST read 0 — the
    /// expired frames are not wire loss and must not collapse the AIMD.
    #[test]
    fn frame_seq_gaps_with_contiguous_transport_seq_are_not_loss() {
        let mut w = VideoReceptionWindow::new(0);
        // frame_seq jumps 0,5,10,26,40,... (pacer expired the rest),
        // yet transport_seq is 0,1,2,3,4,... — every fragment that WAS
        // sent arrived.
        let frame_seqs = [0u32, 5, 10, 26, 40, 41, 90, 130, 131, 200, 260, 261];
        let ack = feed(&mut w, 0, &frame_seqs, 800, 0, 100)
            .expect("12×100ms spans the window");
        assert_eq!(
            ack.loss_q8, 0,
            "sender-side frame expiry is NOT network loss — the death-spiral guard"
        );
        // The window closes at the fragment where elapsed first hits
        // ACK_WINDOW_MS (index 10 at t=1000ms, frame_seq 260); 261 lands
        // in the next window. The ack carries the highest frame_seq
        // observed within the closed window.
        assert_eq!(ack.last_frame_seq, 260, "highest frame_seq in the window is carried through");
    }

    #[test]
    fn real_transport_seq_gaps_are_loss() {
        let mut w = VideoReceptionWindow::new(0);
        // Sent transport seq 0..20, but 10 of them never arrived
        // (dropped on the wire): receive only the even ones.
        let mut last = None;
        for seq in (0..20u32).filter(|s| s % 2 == 0) {
            last = w.observe(seq, seq, SID, "ch1", 500, seq * 60);
        }
        // Force the window closed with one more past the boundary.
        let ack = last
            .or_else(|| w.observe(20, 20, SID, "ch1", 500, ACK_WINDOW_MS + 1))
            .unwrap();
        // ~half the sent sequence is missing → substantial loss.
        assert!(
            ack.loss_q8 > 100,
            "half the transport sequence missing must read as heavy loss: {}",
            ack.loss_q8
        );
    }

    #[test]
    fn goodput_is_measured() {
        let mut w = VideoReceptionWindow::new(0);
        // 13 fragments × 1000 bytes over ~1200 ms ≈ 13000 B ≈ 104 kbit
        // in ~1.2 s ≈ ~86 kbps.
        let seqs: Vec<u32> = (0..13).collect();
        let ack = feed(&mut w, 0, &seqs, 1000, 0, 100).expect("spans window");
        assert!(ack.kbps > 50 && ack.kbps < 130, "measured kbps: {}", ack.kbps);
    }

    #[test]
    fn pacer_restart_does_not_manufacture_loss() {
        let mut w = VideoReceptionWindow::new(0);
        // A long run at high transport_seq, then the sender restarts and
        // begins again at 0 — the jump must resync, not read as ~4 billion
        // lost packets.
        for i in 0..30u32 {
            w.observe(100_000 + i, i, SID, "ch1", 400, i * 10);
        }
        // Restart: seq drops to 0. First post-restart window close.
        let mut ack = None;
        for i in 0..40u32 {
            if let Some(a) = w.observe(i, i, SID, "ch1", 400, 1_000 + i * 40) {
                ack = Some(a);
            }
        }
        let ack = ack.expect("post-restart window elapses");
        assert_eq!(ack.loss_q8, 0, "a pacer restart is a resync, not loss");
    }
}
