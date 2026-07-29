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
//! NOT thread-safe. Must be accessed from a single task (the read task
//! on the receive side). The read task processes frames sequentially
//! from the socket — the replay check happens before dispatch, so it
//! is single-threaded by design.

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
    /// Total nonces accepted since construction. For diagnostics.
    accepted_count: u64,
}

/// Reason a nonce was rejected by the replay filter.
#[derive(Debug, Clone, Copy)]
pub enum ReplayRejection {
    /// The nonce was already accepted (bit set in bitmap).
    Duplicate { nonce: u64, highest: u64, age: usize, accepted_count: u64 },
    /// The nonce is too old — outside the sliding window.
    TooOld { nonce: u64, highest: u64, age: usize, accepted_count: u64 },
}

impl ReplayFilter {
    /// Create a new replay filter with an empty window.
    pub fn new() -> Self {
        Self {
            highest: 0,
            bitmap: [0u64; WINDOW_WORDS],
            accepted_count: 0,
        }
    }

    /// Check and accept a nonce.
    ///
    /// Returns `Ok(())` if the nonce is valid (not replayed, within window).
    /// Returns `Err(ReplayRejection)` with full diagnostic state if rejected.
    ///
    /// On `Ok`, the nonce is marked as accepted and will be rejected
    /// on any subsequent call with the same value.
    pub fn check_and_accept(&mut self, nonce: u64) -> Result<(), ReplayRejection> {
        if nonce > self.highest {
            // Advance the window.
            #[allow(clippy::cast_possible_truncation)]
            let shift = (nonce - self.highest) as usize;
            if shift >= WINDOW_SIZE {
                // Entire window is stale; reset.
                self.bitmap = [0u64; WINDOW_WORDS];
            } else {
                self.shift_left(shift);
            }
            self.highest = nonce;
            // Mark the current nonce as accepted (bit 0 = highest).
            self.bitmap[0] |= 1;
            self.accepted_count += 1;
            return Ok(());
        }

        #[allow(clippy::cast_possible_truncation)]
        let age = (self.highest - nonce) as usize;
        if age >= WINDOW_SIZE {
            tracing::error!(nonce, highest = self.highest, age, accepted_count = self.accepted_count, "ReplayFilter: REJECTED TooOld");
            return Err(ReplayRejection::TooOld {
                nonce, highest: self.highest, age, accepted_count: self.accepted_count,
            });
        }

        let word = age / 64;
        let bit = age % 64;
        if self.bitmap[word] & (1u64 << bit) != 0 {
            tracing::error!(nonce, highest = self.highest, age, accepted_count = self.accepted_count, "ReplayFilter: REJECTED Duplicate");
            return Err(ReplayRejection::Duplicate {
                nonce, highest: self.highest, age, accepted_count: self.accepted_count,
            });
        }

        // Accept and mark.
        self.bitmap[word] |= 1u64 << bit;
        self.accepted_count += 1;
        Ok(())
    }

    /// Total nonces accepted since construction.
    pub fn accepted_count(&self) -> u64 {
        self.accepted_count
    }

    /// The highest nonce seen so far.
    pub fn highest(&self) -> u64 {
        self.highest
    }

    /// Shift the bitmap left by `shift` bit positions.
    fn shift_left(&mut self, shift: usize) {
        let word_shift = shift / 64;
        let bit_shift = shift % 64;

        if word_shift > 0 {
            for i in (0..WINDOW_WORDS).rev() {
                let src = i.checked_sub(word_shift);
                self.bitmap[i] = src.map_or(0, |s| self.bitmap[s]);
            }
        }

        if bit_shift > 0 {
            for i in (0..WINDOW_WORDS).rev() {
                self.bitmap[i] <<= bit_shift;
                if i > 0 {
                    self.bitmap[i] |= self.bitmap[i - 1] >> (64 - bit_shift);
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

    fn ok(rf: &mut ReplayFilter, nonce: u64) {
        rf.check_and_accept(nonce).unwrap_or_else(|e| {
            panic!("nonce {nonce} should be accepted, got {e:?}");
        });
    }

    fn err(rf: &mut ReplayFilter, nonce: u64) {
        assert!(
            rf.check_and_accept(nonce).is_err(),
            "nonce {nonce} should be rejected"
        );
    }

    #[test]
    fn sequential_nonces_accepted() {
        let mut rf = ReplayFilter::new();
        for i in 0..100 { ok(&mut rf, i); }
    }

    #[test]
    fn replay_rejected() {
        let mut rf = ReplayFilter::new();
        ok(&mut rf, 42);
        err(&mut rf, 42);
    }

    #[test]
    fn out_of_order_within_window() {
        let mut rf = ReplayFilter::new();
        ok(&mut rf, 100);
        ok(&mut rf, 98);
        ok(&mut rf, 99);
        err(&mut rf, 98);
    }

    #[test]
    fn too_old_rejected() {
        let mut rf = ReplayFilter::new();
        ok(&mut rf, 2000);
        err(&mut rf, 0);
    }

    #[test]
    fn large_jump_resets_window() {
        let mut rf = ReplayFilter::new();
        ok(&mut rf, 0);
        ok(&mut rf, 5000);
        err(&mut rf, 0);
        ok(&mut rf, 4999);
        err(&mut rf, 4999);
    }

    #[test]
    fn window_boundary_exact() {
        let mut rf = ReplayFilter::new();
        ok(&mut rf, 1023);
        ok(&mut rf, 0);
        ok(&mut rf, 1024);
        err(&mut rf, 0);
    }

    #[test]
    fn cross_word_boundary() {
        let mut rf = ReplayFilter::new();
        ok(&mut rf, 64);
        ok(&mut rf, 0);
        err(&mut rf, 0);
    }

    #[test]
    fn many_out_of_order() {
        let mut rf = ReplayFilter::new();
        for i in (0..100).rev() { ok(&mut rf, i); }
        for i in 0..100 { err(&mut rf, i); }
    }
}
