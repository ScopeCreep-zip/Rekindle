//! M9.3 — sliding-window anti-replay for voice packets.
//!
//! Standard pattern (RFC 4302/4303 §3.4.3, SRTP, WireGuard noise/ratchet):
//! track a high-water sequence number plus a bitmap of recently-accepted
//! sequences. Reject any packet whose sequence has already been accepted
//! (replay) or which falls below the window (too-late reorder).
//!
//! Maintained per-peer by the receive loop. Without this, a malicious
//! relay could capture and replay an accepted voice packet — even with
//! signatures intact, the receiver would feed duplicate audio into the
//! jitter buffer. The current `JitterBuffer` drops `seq < next_playback`
//! but only AFTER an envelope has been accepted; once a packet has been
//! popped, replays are accepted again as fresh-from-network. The replay
//! window closes that gap by rejecting at the network ingress, before
//! the jitter buffer gets a chance to look at the packet.

/// Window width in sequence numbers. 256 chosen as a multiple of the
/// 64-bit bitmap word size and large enough to absorb realistic
/// reordering bursts (a 5-second LTE handover at 50 packets/sec ≈ 250).
pub const WINDOW_SIZE: u32 = 256;

/// Number of `u64` words in the bitmap.
const WINDOW_WORDS: usize = (WINDOW_SIZE as usize).div_ceil(64);

/// Per-peer voice replay-protection window.
#[derive(Debug, Clone)]
pub struct VoiceSeqWindow {
    /// Highest accepted sequence number. Bit 0 of `bitmap[0]`
    /// corresponds to this number; bit `i` of `bitmap[i/64]` (mod 64)
    /// corresponds to `high_water - i`.
    high_water: Option<u32>,
    bitmap: [u64; WINDOW_WORDS],
}

impl Default for VoiceSeqWindow {
    fn default() -> Self {
        Self::new()
    }
}

impl VoiceSeqWindow {
    pub fn new() -> Self {
        Self {
            high_water: None,
            bitmap: [0; WINDOW_WORDS],
        }
    }

    /// Try to accept `seq`. Returns `true` if novel; `false` if it's a
    /// replay of an already-accepted sequence or has fallen below the
    /// window. The window is updated only on accept.
    pub fn check_and_insert(&mut self, seq: u32) -> bool {
        let Some(high) = self.high_water else {
            // First packet — accept and seed the window.
            self.high_water = Some(seq);
            self.bitmap[0] = 1;
            return true;
        };

        if seq > high {
            // Forward jump — shift the window left by `delta` positions
            // and set bit 0 to mark `seq` as accepted.
            let delta = seq - high;
            self.shift_window(delta);
            self.bitmap[0] |= 1;
            self.high_water = Some(seq);
            return true;
        }

        let offset = high - seq;
        if offset >= WINDOW_SIZE {
            // Below the window — too late to verify, drop. Even if this
            // is a legitimate reordered packet, accepting it would give
            // an attacker an unbounded replay budget.
            return false;
        }

        let word = (offset / 64) as usize;
        let bit = offset % 64;
        let mask = 1u64 << bit;
        if self.bitmap[word] & mask != 0 {
            return false; // Replay.
        }
        self.bitmap[word] |= mask;
        true
    }

    /// Shift the window left by `delta` positions. Bits that fall off
    /// the high end are discarded (those sequences were never seen).
    fn shift_window(&mut self, delta: u32) {
        if delta as usize >= WINDOW_SIZE as usize {
            // Whole window slid out — wipe it.
            self.bitmap = [0; WINDOW_WORDS];
            return;
        }
        let word_shift = (delta / 64) as usize;
        let bit_shift = delta % 64;

        if word_shift > 0 {
            for i in (word_shift..WINDOW_WORDS).rev() {
                self.bitmap[i] = self.bitmap[i - word_shift];
            }
            for word in self.bitmap.iter_mut().take(word_shift) {
                *word = 0;
            }
        }

        if bit_shift > 0 {
            let carry_mask = (1u64 << bit_shift) - 1;
            let mut carry: u64 = 0;
            for word in &mut self.bitmap {
                let new_carry = (*word >> (64 - bit_shift)) & carry_mask;
                *word = (*word << bit_shift) | carry;
                carry = new_carry;
            }
        }
    }

    /// Highest accepted sequence number, if any. Useful for tests and
    /// for stale-peer pruning ("we've seen no packets above N for T
    /// seconds → reset the window").
    pub fn high_water(&self) -> Option<u32> {
        self.high_water
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_packet_accepted() {
        let mut w = VoiceSeqWindow::new();
        assert!(w.check_and_insert(42));
        assert_eq!(w.high_water(), Some(42));
    }

    #[test]
    fn duplicate_rejected_immediately() {
        let mut w = VoiceSeqWindow::new();
        assert!(w.check_and_insert(10));
        assert!(!w.check_and_insert(10));
    }

    #[test]
    fn out_of_order_within_window_accepted_once() {
        let mut w = VoiceSeqWindow::new();
        assert!(w.check_and_insert(10));
        assert!(w.check_and_insert(20));
        // A reordered packet 15 within the window — accept once.
        assert!(w.check_and_insert(15));
        // Replay of 15 — reject.
        assert!(!w.check_and_insert(15));
    }

    #[test]
    fn below_window_rejected() {
        let mut w = VoiceSeqWindow::new();
        assert!(w.check_and_insert(1000));
        // Sequence 1 is far below the window — reject (could not be a
        // late-but-novel reorder; window only protects 256 back).
        assert!(!w.check_and_insert(1));
    }

    #[test]
    fn forward_jump_resets_far_window() {
        let mut w = VoiceSeqWindow::new();
        assert!(w.check_and_insert(10));
        // Big jump — old bits are all cleared, only the new high_water
        // bit is set.
        assert!(w.check_and_insert(10 + WINDOW_SIZE * 5));
        // Replay of 10 should still be rejected (it's now below window).
        assert!(!w.check_and_insert(10));
    }

    #[test]
    fn replay_at_high_water_rejected() {
        let mut w = VoiceSeqWindow::new();
        assert!(w.check_and_insert(100));
        assert!(w.check_and_insert(101));
        // Replay of 101 (== current high_water) must be rejected.
        assert!(!w.check_and_insert(101));
    }

    #[test]
    fn many_in_order_packets() {
        let mut w = VoiceSeqWindow::new();
        for seq in 0..10_000u32 {
            assert!(w.check_and_insert(seq), "seq {seq}");
        }
        // Verify replays of recent sequences are rejected.
        assert!(!w.check_and_insert(9_999));
        assert!(!w.check_and_insert(9_500));
        // And below-window sequences too.
        assert!(!w.check_and_insert(0));
    }

    #[test]
    fn window_edge_boundary() {
        let mut w = VoiceSeqWindow::new();
        assert!(w.check_and_insert(WINDOW_SIZE));
        // Sequence at exactly `WINDOW_SIZE - 1` ago = offset = WINDOW_SIZE - 1
        // is the oldest sequence still in the window — accept.
        assert!(w.check_and_insert(1));
        // Sequence at offset == WINDOW_SIZE — outside the window, reject.
        assert!(!w.check_and_insert(0));
    }
}
