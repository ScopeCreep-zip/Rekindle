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
use std::sync::Arc;

use parking_lot::Mutex;

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

/// One host's indicator: its window and the host's clock. The Host and,
/// until F1, the desktop's own Veilid path each keep one and run the same
/// rule through it (plan E7.2).
pub struct Strength {
    window: Mutex<StrengthWindow>,
    clock: Arc<dyn rekindle_utils::Clock>,
}

/// What one attachment report asks of the host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Report {
    /// The bars to publish now, when the shown value changed.
    pub publish: Option<u8>,
    /// A fall is held: run [`Strength::settle_held`] and publish what it
    /// returns.
    pub hold: bool,
}

impl Strength {
    #[must_use]
    pub fn new(clock: Arc<dyn rekindle_utils::Clock>) -> Self {
        Self {
            window: Mutex::default(),
            clock,
        }
    }

    /// The bars shown now, for a window that opens after the last event.
    #[must_use]
    pub fn shown(&self) -> u8 {
        self.window.lock().shown()
    }

    /// Veilid reported `attachment` (its `AttachmentState` string).
    #[must_use]
    pub fn report(&self, attachment: &str) -> Report {
        let bars =
            rekindle_types::notification::AttachmentState::from_veilid_string(attachment).bars();
        let mut window = self.window.lock();
        let publish = window.sample(self.clock.now_ms(), bars);
        Report {
            publish,
            hold: window.holding(),
        }
    }

    /// Wait out a held fall's hold, then re-read the window: the bars to
    /// publish when the shown value changed.
    pub async fn settle_held(&self) -> Option<u8> {
        tokio::time::sleep(std::time::Duration::from_millis(HOLD_MS)).await;
        self.window.lock().settle(self.clock.now_ms())
    }
}

#[cfg(test)]
#[path = "strength_tests.rs"]
mod tests;
