//! Sliding-window nonce replay filter.
//!
//! Prevents an attacker from resending previously-observed encrypted
//! frames. The filter tracks a 1024-nonce window centered on the
//! highest accepted nonce. Nonces below the window floor are rejected
//! (too old). Nonces within the window are accepted only if their
//! bitmap bit is not already set (not replayed).
//!
//! # Thread safety
//!
//! NOT thread-safe. Must be accessed from a single task (the bulk
//! dispatcher on the receive side). The dispatcher is a sequential
//! frame processor — it reads frames from the socket in order and
//! dispatches each to a rayon worker. The replay check happens
//! before dispatch, so it is single-threaded by design.

/// Window size in nonces. Nonces more than this far behind the
/// highest accepted nonce are unconditionally rejected.
const WINDOW_SIZE: usize = 1024;

/// Number of u64 words in the bitmap.
const WINDOW_WORDS: usize = WINDOW_SIZE / 64;

/// Sliding-window replay filter for nonce values.
pub struct ReplayFilter {
    /// The highest accepted nonce value.
    highest: u64,
    /// Bitmap of accepted nonces within `[highest - WINDOW + 1, highest]`.
    /// Bit position `i` in word `i/64` corresponds to nonce `highest - i`.
    bitmap: [u64; WINDOW_WORDS],
}

impl ReplayFilter {
    /// Create a new replay filter with an empty window.
    pub fn new() -> Self {
        Self {
            highest: 0,
            bitmap: [0u64; WINDOW_WORDS],
        }
    }

    /// Check and accept a nonce.
    ///
    /// Returns `true` if the nonce is valid (not replayed, within window).
    /// Returns `false` if the nonce is a replay or too old.
    ///
    /// On `true`, the nonce is marked as accepted and will be rejected
    /// on any subsequent call with the same value.
    pub fn check_and_accept(&mut self, nonce: u64) -> bool {
        if nonce > self.highest {
            // Advance the window.
            #[allow(clippy::cast_possible_truncation)] // WINDOW_SIZE is 1024; shift > 1024 resets bitmap
            let shift = (nonce - self.highest) as usize;
            if shift >= WINDOW_SIZE {
                // Entire window is stale; reset.
                self.bitmap = [0u64; WINDOW_WORDS];
            } else {
                self.shift_right(shift);
            }
            self.highest = nonce;
            // Mark the current nonce as accepted (bit 0 = highest).
            self.bitmap[0] |= 1;
            return true;
        }

        #[allow(clippy::cast_possible_truncation)] // age < WINDOW_SIZE (1024) by the check below
        let age = (self.highest - nonce) as usize;
        if age >= WINDOW_SIZE {
            // Too old — outside the window.
            return false;
        }

        let word = age / 64;
        let bit = age % 64;
        if self.bitmap[word] & (1u64 << bit) != 0 {
            // Already accepted — replay.
            return false;
        }

        // Accept and mark.
        self.bitmap[word] |= 1u64 << bit;
        true
    }

    /// Shift the bitmap right by `shift` bit positions.
    ///
    /// This moves old nonces toward higher bit positions (and
    /// eventually off the end of the bitmap), making room for new
    /// nonces at bit position 0.
    fn shift_right(&mut self, shift: usize) {
        let word_shift = shift / 64;
        let bit_shift = shift % 64;

        if word_shift > 0 {
            // Shift entire words first.
            for i in (0..WINDOW_WORDS).rev() {
                let src = if i >= word_shift { i - word_shift } else { WINDOW_WORDS };
                if src < WINDOW_WORDS {
                    self.bitmap[i] = self.bitmap[src];
                } else {
                    self.bitmap[i] = 0;
                }
            }
        }

        if bit_shift > 0 {
            // Shift bits within words. Process from high to low so
            // we don't overwrite source data before reading it.
            for i in (0..WINDOW_WORDS).rev() {
                self.bitmap[i] >>= bit_shift;
                if i > 0 {
                    self.bitmap[i] |= self.bitmap[i - 1] << (64 - bit_shift);
                }
            }
        }
    }
}

impl Default for ReplayFilter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sequential_nonces_accepted() {
        let mut rf = ReplayFilter::new();
        for i in 0..100 {
            assert!(rf.check_and_accept(i), "nonce {i} should be accepted");
        }
    }

    #[test]
    fn replay_rejected() {
        let mut rf = ReplayFilter::new();
        assert!(rf.check_and_accept(42));
        assert!(!rf.check_and_accept(42)); // replay
    }

    #[test]
    fn out_of_order_within_window() {
        let mut rf = ReplayFilter::new();
        assert!(rf.check_and_accept(100));
        assert!(rf.check_and_accept(98)); // 2 behind, within window
        assert!(rf.check_and_accept(99)); // 1 behind, within window
        assert!(!rf.check_and_accept(98)); // replay of 98
    }

    #[test]
    fn too_old_rejected() {
        let mut rf = ReplayFilter::new();
        assert!(rf.check_and_accept(2000));
        assert!(!rf.check_and_accept(0)); // 2000 behind, outside window
    }

    #[test]
    fn large_jump_resets_window() {
        let mut rf = ReplayFilter::new();
        assert!(rf.check_and_accept(0));
        assert!(rf.check_and_accept(5000)); // large jump
        assert!(!rf.check_and_accept(0)); // too old
        assert!(rf.check_and_accept(4999)); // within new window
        assert!(!rf.check_and_accept(4999)); // replay
    }

    #[test]
    fn window_boundary_exact() {
        let mut rf = ReplayFilter::new();
        assert!(rf.check_and_accept(1023));

        // Nonce 0 is exactly at the window boundary (age = 1023 < 1024).
        assert!(rf.check_and_accept(0));

        // Now advance to 1024. Nonce 0 is now age=1024 = WINDOW_SIZE.
        assert!(rf.check_and_accept(1024));
        assert!(!rf.check_and_accept(0)); // now outside window
    }

    #[test]
    fn cross_word_boundary() {
        let mut rf = ReplayFilter::new();
        // Accept nonce 64, then nonce 0 (age = 64, crosses word boundary).
        assert!(rf.check_and_accept(64));
        assert!(rf.check_and_accept(0)); // word 1, bit 0
        assert!(!rf.check_and_accept(0)); // replay
    }

    #[test]
    fn many_out_of_order() {
        let mut rf = ReplayFilter::new();
        // Accept nonces in reverse order within a 100-nonce range.
        for i in (0..100).rev() {
            assert!(
                rf.check_and_accept(i),
                "nonce {i} should be accepted (reverse order)"
            );
        }
        // All should now be rejected as replays.
        for i in 0..100 {
            assert!(
                !rf.check_and_accept(i),
                "nonce {i} should be rejected (replay)"
            );
        }
    }

    #[test]
    fn shift_64_preserves_bits() {
        let mut rf = ReplayFilter::new();
        assert!(rf.check_and_accept(0));
        assert!(rf.check_and_accept(64)); // shift right by 64
        // Nonce 0 is at age 64, which means word=1, bit=0.
        // It was set before the shift and should still be set.
        assert!(!rf.check_and_accept(0)); // replay — correctly rejected
    }

    #[test]
    fn shift_63_same_word_boundary() {
        let mut rf = ReplayFilter::new();
        assert!(rf.check_and_accept(0));
        assert!(rf.check_and_accept(63));
        assert!(!rf.check_and_accept(0));
    }

    #[test]
    fn shift_65_cross_word_plus_one() {
        let mut rf = ReplayFilter::new();
        assert!(rf.check_and_accept(0));
        assert!(rf.check_and_accept(65));
        assert!(!rf.check_and_accept(0));
    }

    #[test]
    fn shift_127_near_second_word_boundary() {
        let mut rf = ReplayFilter::new();
        assert!(rf.check_and_accept(0));
        assert!(rf.check_and_accept(127));
        assert!(!rf.check_and_accept(0));
    }

    #[test]
    fn shift_128_exact_two_words() {
        let mut rf = ReplayFilter::new();
        assert!(rf.check_and_accept(0));
        assert!(rf.check_and_accept(128));
        assert!(!rf.check_and_accept(0));
    }

    #[test]
    fn shift_1023_window_edge_exact() {
        let mut rf = ReplayFilter::new();
        assert!(rf.check_and_accept(0));
        assert!(rf.check_and_accept(1023));
        assert!(!rf.check_and_accept(0));
        assert!(rf.check_and_accept(1024));
        assert!(!rf.check_and_accept(0));
    }
}
