//! The network indicator's bars, smoothed (plan E7.2, live finding L8).
//!
//! Veilid reports attachment as "signal strength" (0.5.8
//! `veilid_api/types/veilid_state.rs:3-24`) and sends an update only when it
//! changes. A short dip flickered the indicator, so the shown bars are the
//! highest in effect over the last [`HOLD_MS`]: a rise shows at once, a fall
//! only once it has lasted the hold. The Host keeps one window and
//! publishes `NetworkEvent::Strength` when the shown value changes, so every
//! frontend shows the same bars. `public_internet_ready` gates operations
//! only; it never moves the bars.

use std::collections::VecDeque;

/// How long a lower strength must last before the bars fall.
pub const HOLD_MS: u64 = 5_000;

/// The attachment samples of the last [`HOLD_MS`] and the bars shown.
#[derive(Debug, Default)]
pub struct StrengthWindow {
    /// `(at_ms, bars)`, oldest first; the oldest may predate the window
    /// (it was in effect at the window's start).
    samples: VecDeque<(u64, u8)>,
    shown: u8,
}

impl StrengthWindow {
    /// The bars shown now.
    #[must_use]
    pub fn shown(&self) -> u8 {
        self.shown
    }

    /// Veilid reported `bars` at `now_ms`. The bars to publish when the
    /// shown value changed.
    pub fn sample(&mut self, now_ms: u64, bars: u8) -> Option<u8> {
        self.samples.push_back((now_ms, bars));
        self.settle(now_ms)
    }

    /// Whether a fall is being held: the latest sample is below the shown
    /// bars, so [`Self::settle`] is due [`HOLD_MS`] after it.
    #[must_use]
    pub fn holding(&self) -> bool {
        self.samples.back().is_some_and(|&(_, b)| b < self.shown)
    }

    /// Re-read the window at `now_ms` (a held fall's hold ended). The bars
    /// to publish when the shown value changed.
    pub fn settle(&mut self, now_ms: u64) -> Option<u8> {
        let start = now_ms.saturating_sub(HOLD_MS);
        // Keep the newest sample at or before the window's start: it was in
        // effect then. Older ones no longer count.
        while self.samples.len() > 1 && self.samples[1].0 <= start {
            self.samples.pop_front();
        }
        let bars = self.samples.iter().map(|&(_, b)| b).max().unwrap_or(0);
        (bars != self.shown).then(|| {
            self.shown = bars;
            bars
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// L8: a 2 s dip does not move the bars; a rise shows at once; a fall
    /// that lasts the hold shows when it ends.
    #[test]
    fn a_short_dip_holds_and_a_lasting_fall_shows_after_the_hold() {
        let mut w = StrengthWindow::default();
        assert_eq!(w.sample(0, 4), Some(4), "a rise shows at once");
        assert_eq!(w.sample(1_000, 1), None, "the dip is held");
        assert!(w.holding());
        assert_eq!(w.sample(3_000, 4), None, "back within the hold");
        assert!(!w.holding());
        assert_eq!(w.settle(3_000 + HOLD_MS), None, "the dip never shows");

        assert_eq!(w.sample(10_000, 2), None);
        assert_eq!(w.settle(10_000 + HOLD_MS - 1), None, "still held");
        assert_eq!(w.settle(10_000 + HOLD_MS), Some(2), "the fall shows");
        assert_eq!(w.sample(20_000, 5), Some(5));
        assert_eq!(w.shown(), 5);
    }
}
