//! Lip sync, measured at the output (plan E4.3 Q0).
//!
//! For one sender, each played audio frame gives an *audio offset*: the
//! local time its samples reach the speaker minus the sender's capture
//! time. Each rendered video frame gives a *video offset* the same way.
//! Both carry the same unknown clock offset between the machines, so their
//! difference is the skew the listener sees:
//!
//! `skew = video offset − audio offset`
//!
//! Positive means the picture is later than the sound (audio ahead).
//!
//! The bounds are ITU-T P.1305 §9.1 / ITU-R BT.1359: audio at most 90 ms
//! ahead of the video and at most 185 ms behind it.

use std::collections::VecDeque;

/// Audio may lead the video by at most this, ms.
pub const AUDIO_LEAD_LIMIT_MS: i64 = 90;
/// Audio may lag the video by at most this, ms.
pub const AUDIO_LAG_LIMIT_MS: i64 = 185;

/// Offsets kept per kind: about 5 s of audio (20 ms frames) or video.
const OFFSET_WINDOW: usize = 250;
/// Skew samples a call keeps (one per sample period): about three hours
/// at 5 s.
const MAX_SKEWS: usize = 2_160;

/// The call's skew distribution so far.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SyncSummary {
    pub samples: usize,
    pub skew_p50_ms: i64,
    /// The 5th and 95th percentiles: the spread the listener sees.
    pub skew_p5_ms: i64,
    pub skew_p95_ms: i64,
    /// Samples outside the P.1305 bounds.
    pub outside_bounds: usize,
}

/// Skew between one sender's audio and video.
#[derive(Debug, Default)]
pub struct SyncTracker {
    audio: VecDeque<i64>,
    video: VecDeque<i64>,
    skews: VecDeque<i64>,
}

fn push(window: &mut VecDeque<i64>, value: i64, cap: usize) {
    window.push_back(value);
    if window.len() > cap {
        window.pop_front();
    }
}

fn median(values: &VecDeque<i64>) -> Option<i64> {
    if values.is_empty() {
        return None;
    }
    let mut sorted: Vec<i64> = values.iter().copied().collect();
    sorted.sort_unstable();
    Some(sorted[sorted.len() / 2])
}

fn percentile(sorted: &[i64], pct: usize) -> i64 {
    sorted[(sorted.len() - 1) * pct / 100]
}

/// Whether `skew_ms` is within the P.1305 bounds.
#[must_use]
pub fn within_bounds(skew_ms: i64) -> bool {
    (-AUDIO_LAG_LIMIT_MS..=AUDIO_LEAD_LIMIT_MS).contains(&skew_ms)
}

impl SyncTracker {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// An audio frame reached the speaker `offset_ms` after (sender clock)
    /// its capture.
    pub fn note_audio(&mut self, offset_ms: i64) {
        push(&mut self.audio, offset_ms, OFFSET_WINDOW);
    }

    /// A video frame was rendered `offset_ms` after (sender clock) its
    /// capture.
    pub fn note_video(&mut self, offset_ms: i64) {
        push(&mut self.video, offset_ms, OFFSET_WINDOW);
    }

    /// Take one skew sample from the current windows (medians, so a single
    /// late frame does not move it), and start fresh windows. `None` until
    /// both kinds have been seen since the last sample.
    pub fn sample(&mut self) -> Option<i64> {
        let skew = median(&self.video)? - median(&self.audio)?;
        self.audio.clear();
        self.video.clear();
        push(&mut self.skews, skew, MAX_SKEWS);
        Some(skew)
    }

    #[must_use]
    pub fn summary(&self) -> SyncSummary {
        if self.skews.is_empty() {
            return SyncSummary::default();
        }
        let mut sorted: Vec<i64> = self.skews.iter().copied().collect();
        sorted.sort_unstable();
        SyncSummary {
            samples: sorted.len(),
            skew_p50_ms: percentile(&sorted, 50),
            skew_p5_ms: percentile(&sorted, 5),
            skew_p95_ms: percentile(&sorted, 95),
            outside_bounds: sorted.iter().filter(|s| !within_bounds(**s)).count(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_clock_offset_cancels() {
        // The receiver's clock is 10 s ahead; audio plays 120 ms after
        // capture, video renders 180 ms after: video 60 ms late.
        let mut s = SyncTracker::new();
        for _ in 0..50 {
            s.note_audio(10_000 + 120);
            s.note_video(10_000 + 180);
        }
        assert_eq!(s.sample(), Some(60));
        assert!(within_bounds(60));
    }

    #[test]
    fn needs_both_kinds_each_sample() {
        let mut s = SyncTracker::new();
        s.note_audio(100);
        assert_eq!(s.sample(), None);
        s.note_video(150);
        assert_eq!(s.sample(), Some(50));
        s.note_video(150);
        assert_eq!(s.sample(), None, "audio window started fresh");
    }

    #[test]
    fn bounds_follow_p1305() {
        assert!(within_bounds(90));
        assert!(!within_bounds(91), "audio 91 ms ahead");
        assert!(within_bounds(-185));
        assert!(!within_bounds(-186), "audio 186 ms behind");
    }

    #[test]
    fn summary_reports_spread_and_violations() {
        let mut s = SyncTracker::new();
        for skew in [10, 20, 30, 200, -300] {
            s.note_audio(0);
            s.note_video(skew);
            s.sample();
        }
        let sum = s.summary();
        assert_eq!(sum.samples, 5);
        assert_eq!(sum.skew_p50_ms, 20);
        assert_eq!(sum.outside_bounds, 2);
    }
}
