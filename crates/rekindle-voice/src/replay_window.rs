//! M9.3 — sliding-window anti-replay for voice frames, keyed on the
//! SFrame CTR (RFC 9605 §9.3 suggests exactly this: RFC 3711 §3.3.2's
//! counter window over CTR).
//!
//! Standard pattern (RFC 4302/4303 §3.4.3, SRTP, WireGuard noise/ratchet):
//! track a high-water counter plus a bitmap of recently-accepted
//! counters. Reject any frame whose counter has already been accepted
//! (replay) or which falls below the window (too-late reorder).
//!
//! Maintained per sender key by the frame opener. Without this, a malicious
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
pub const WINDOW_SIZE: u64 = 256;

/// Number of `u64` words in the bitmap.
const WINDOW_WORDS: usize = 4;
const _: () = assert!(WINDOW_WORDS * 64 == 256 && WINDOW_SIZE == 256);

/// Per-sender-key replay-protection window over SFrame CTR values.
#[derive(Debug, Clone)]
pub struct CtrWindow {
    /// Highest accepted sequence number. Bit 0 of `bitmap[0]`
    /// corresponds to this number; bit `i` of `bitmap[i/64]` (mod 64)
    /// corresponds to `high_water - i`.
    high_water: Option<u64>,
    bitmap: [u64; WINDOW_WORDS],
}

impl Default for CtrWindow {
    fn default() -> Self {
        Self::new()
    }
}

impl CtrWindow {
    pub fn new() -> Self {
        Self {
            high_water: None,
            bitmap: [0; WINDOW_WORDS],
        }
    }

    /// Try to accept `seq`. Returns `true` if novel; `false` if it's a
    /// replay of an already-accepted sequence or has fallen below the
    /// window. The window is updated only on accept.
    pub fn check_and_insert(&mut self, seq: u64) -> bool {
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

        // `offset < WINDOW_SIZE` = 256, so both fit.
        let word = usize::try_from(offset / 64).unwrap_or(WINDOW_WORDS - 1);
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
    fn shift_window(&mut self, delta: u64) {
        if delta >= WINDOW_SIZE {
            // Whole window slid out — wipe it.
            self.bitmap = [0; WINDOW_WORDS];
            return;
        }
        // `delta < WINDOW_SIZE` here, so this is at most 3.
        let word_shift = usize::try_from(delta / 64).unwrap_or(WINDOW_WORDS);
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
    pub fn high_water(&self) -> Option<u64> {
        self.high_water
    }
}

#[cfg(test)]
#[path = "replay_window_tests.rs"]
mod tests;
