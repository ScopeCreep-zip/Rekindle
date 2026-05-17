//! Sliding-window nonce replay filter.
//!
//! Tracks a 1024-nonce window. Nonces below the window floor are
//! rejected (too old). Nonces within the window are accepted only
//! if their bitmap bit is not set.
//!
//! NOT thread-safe. Single-threaded by design (the dispatcher
//! processes frames sequentially from the socket read loop).

const WINDOW_SIZE: usize = 1024;
const WINDOW_WORDS: usize = WINDOW_SIZE / 64;

/// Reason a nonce was rejected.
#[derive(Debug, Clone, Copy)]
pub enum ReplayRejection {
    Duplicate { nonce: u64 },
    TooOld { nonce: u64, highest: u64 },
}

/// Sliding-window replay filter.
pub struct ReplayFilter {
    highest: u64,
    bitmap: [u64; WINDOW_WORDS],
    accepted_count: u64,
}

impl ReplayFilter {
    pub fn new() -> Self {
        Self {
            highest: 0,
            bitmap: [0u64; WINDOW_WORDS],
            accepted_count: 0,
        }
    }

    /// Check and accept a nonce. Returns Ok if valid, Err if replay/too-old.
    pub fn check_and_accept(&mut self, nonce: u64) -> Result<(), ReplayRejection> {
        if nonce > self.highest {
            #[allow(clippy::cast_possible_truncation)]
            let shift = (nonce - self.highest) as usize;
            if shift >= WINDOW_SIZE {
                self.bitmap = [0u64; WINDOW_WORDS];
            } else {
                self.shift_left(shift);
            }
            self.highest = nonce;
            self.bitmap[0] |= 1;
            self.accepted_count += 1;
            return Ok(());
        }

        #[allow(clippy::cast_possible_truncation)]
        let age = (self.highest - nonce) as usize;
        if age >= WINDOW_SIZE {
            return Err(ReplayRejection::TooOld {
                nonce,
                highest: self.highest,
            });
        }

        let word = age / 64;
        let bit = age % 64;
        if self.bitmap[word] & (1u64 << bit) != 0 {
            return Err(ReplayRejection::Duplicate { nonce });
        }

        self.bitmap[word] |= 1u64 << bit;
        self.accepted_count += 1;
        Ok(())
    }

    pub fn accepted_count(&self) -> u64 {
        self.accepted_count
    }

    pub fn highest(&self) -> u64 {
        self.highest
    }

    /// Shift the bitmap LEFT by `shift` positions to make room at bit 0
    /// for the new highest nonce. Older nonces move to higher bit positions.
    fn shift_left(&mut self, shift: usize) {
        let word_shift = shift / 64;
        let bit_shift = shift % 64;

        if word_shift > 0 {
            for i in (0..WINDOW_WORDS).rev() {
                self.bitmap[i] = if i >= word_shift {
                    self.bitmap[i - word_shift]
                } else {
                    0
                };
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

    #[test]
    fn sequential_accepted() {
        let mut rf = ReplayFilter::new();
        for i in 0..100 {
            rf.check_and_accept(i).unwrap();
        }
    }

    #[test]
    fn replay_rejected() {
        let mut rf = ReplayFilter::new();
        rf.check_and_accept(42).unwrap();
        assert!(rf.check_and_accept(42).is_err());
    }

    #[test]
    fn out_of_order_within_window() {
        let mut rf = ReplayFilter::new();
        rf.check_and_accept(100).unwrap();
        rf.check_and_accept(98).unwrap();
        rf.check_and_accept(99).unwrap();
        assert!(rf.check_and_accept(98).is_err());
    }

    #[test]
    fn too_old_rejected() {
        let mut rf = ReplayFilter::new();
        rf.check_and_accept(2000).unwrap();
        assert!(rf.check_and_accept(0).is_err());
    }

    #[test]
    fn large_jump_resets() {
        let mut rf = ReplayFilter::new();
        rf.check_and_accept(0).unwrap();
        rf.check_and_accept(5000).unwrap();
        assert!(rf.check_and_accept(0).is_err());
        rf.check_and_accept(4999).unwrap();
    }
}
