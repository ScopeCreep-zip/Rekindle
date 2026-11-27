//! Rendered-video smoothness — freezes and pauses (plan E4.3 Q0).
//!
//! The definitions are W3C webrtc-stats' (`freezeCount`,
//! `totalFreezesDuration`, `pauseCount`, `totalPausesDuration`) as
//! libwebrtc implements them (`video/video_quality_observer2.cc`):
//!
//! - **freeze**: an interval between two consecutively rendered frames that
//!   is at least `max(3 × avg, avg + 150 ms)`, where `avg` is the mean of
//!   the last 30 intervals *including this one*, once at least 5 have been
//!   seen;
//! - **pause**: more than 5 s since the last rendered frame. A pause is not
//!   a freeze and does not enter the average.
//!
//! The harmonic frame rate is libwebrtc's smoothness figure: rendered time
//! over the sum of squared intervals, so one long stall weighs more than
//! many short ones.
//!
//! Each frame also gives a **sender-to-render offset**: local render time
//! minus the sender's capture time. Clocks are not synchronised, so the
//! offset alone means nothing; its spread is the end-to-end delay variation,
//! and against the audio offset of the same sender it gives lip sync
//! ([`crate::sync`]), where the clock offset cancels.

use std::collections::VecDeque;

/// Intervals in the freeze average (libwebrtc
/// `kAvgInterframeDelaysWindowSizeFrames`).
pub const FREEZE_WINDOW_FRAMES: usize = 30;
/// Intervals needed before a freeze can be detected
/// (`kMinFrameSamplesToDetectFreeze`).
pub const FREEZE_MIN_SAMPLES: usize = 5;
/// The least increase over the average that is a freeze
/// (`kMinIncreaseForFreezeMs`).
pub const FREEZE_MIN_INCREASE_MS: u64 = 150;
/// An interval longer than this is a pause (W3C `pauseCount`).
pub const PAUSE_MS: u64 = 5_000;

/// `value` as `f64`, saturating at `u32::MAX`: counts and milliseconds here
/// stay far below it.
fn as_f64(value: u64) -> f64 {
    f64::from(u32::try_from(value).unwrap_or(u32::MAX))
}

/// One stream's render figures so far.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct RenderSummary {
    pub frames_rendered: u64,
    pub freeze_count: u32,
    pub total_freezes_ms: u64,
    pub pause_count: u32,
    pub total_pauses_ms: u64,
    /// Rendered time, first to last frame, ms.
    pub rendered_span_ms: u64,
    /// Mean frames a second over the span.
    pub fps: f64,
    /// libwebrtc's harmonic frame rate: span / Σ interval².
    pub harmonic_fps: f64,
}

/// Freeze and pause detection over one stream's rendered frames.
#[derive(Debug, Default)]
pub struct RenderTracker {
    intervals: VecDeque<u64>,
    interval_sum: u64,
    first_ms: Option<u64>,
    last_ms: Option<u64>,
    frames: u64,
    freeze_count: u32,
    total_freezes_ms: u64,
    pause_count: u32,
    total_pauses_ms: u64,
    sum_squared_s: f64,
}

impl RenderTracker {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A frame was rendered at local `rendered_ms`. Frames must arrive in
    /// render order; an earlier time than the last is ignored.
    pub fn on_rendered(&mut self, rendered_ms: u64) {
        let Some(last) = self.last_ms else {
            self.first_ms = Some(rendered_ms);
            self.last_ms = Some(rendered_ms);
            self.frames = 1;
            return;
        };
        if rendered_ms < last {
            return;
        }
        let interval = rendered_ms - last;
        self.last_ms = Some(rendered_ms);
        self.frames += 1;
        let secs = as_f64(interval) / 1_000.0;
        self.sum_squared_s += secs * secs;

        if interval > PAUSE_MS {
            self.pause_count += 1;
            self.total_pauses_ms += interval;
            return;
        }
        self.intervals.push_back(interval);
        self.interval_sum += interval;
        if self.intervals.len() > FREEZE_WINDOW_FRAMES {
            if let Some(old) = self.intervals.pop_front() {
                self.interval_sum -= old;
            }
        }
        if self.intervals.len() >= FREEZE_MIN_SAMPLES {
            // Rounded down, as libwebrtc's `GetAverageRoundedDown`.
            let avg = self.interval_sum / self.intervals.len() as u64;
            if interval >= (3 * avg).max(avg + FREEZE_MIN_INCREASE_MS) {
                self.freeze_count += 1;
                self.total_freezes_ms += interval;
            }
        }
    }

    #[must_use]
    pub fn summary(&self) -> RenderSummary {
        let span = match (self.first_ms, self.last_ms) {
            (Some(first), Some(last)) => last - first,
            _ => 0,
        };
        let span_s = as_f64(span) / 1_000.0;
        RenderSummary {
            frames_rendered: self.frames,
            freeze_count: self.freeze_count,
            total_freezes_ms: self.total_freezes_ms,
            pause_count: self.pause_count,
            total_pauses_ms: self.total_pauses_ms,
            rendered_span_ms: span,
            fps: if span_s > 0.0 {
                as_f64(self.frames.saturating_sub(1)) / span_s
            } else {
                0.0
            },
            harmonic_fps: if self.sum_squared_s > 0.0 {
                span_s / self.sum_squared_s
            } else {
                0.0
            },
        }
    }
}

/// What reassembly made of a stream's frames, after FEC: the "post-FEC
/// loss" RFC 8868 §3 asks for.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReceiveCounters {
    /// Frames reassembled.
    pub completed: u64,
    /// Of those, frames that needed parity to complete.
    pub recovered_by_fec: u64,
    /// Frames never completed: gaps in the completed frame sequence.
    pub lost: u64,
    highest_seq: Option<u32>,
}

impl ReceiveCounters {
    /// Frame `frame_seq` was reassembled. A frame older than the highest
    /// seen fills no gap: reassembly evicts a frame it gave up on, so a
    /// late completion is already counted lost.
    pub fn on_completed(&mut self, frame_seq: u32, recovered_by_fec: bool) {
        self.completed += 1;
        if recovered_by_fec {
            self.recovered_by_fec += 1;
        }
        match self.highest_seq {
            None => self.highest_seq = Some(frame_seq),
            Some(highest) => {
                let ahead = frame_seq.wrapping_sub(highest);
                if ahead != 0 && ahead < u32::MAX / 2 {
                    self.lost += u64::from(ahead - 1);
                    self.highest_seq = Some(frame_seq);
                }
            }
        }
    }

    /// Lost frames over frames expected, 0–1.
    #[must_use]
    pub fn loss_fraction(&self) -> f64 {
        let expected = self.completed + self.lost;
        if expected == 0 {
            0.0
        } else {
            as_f64(self.lost) / as_f64(expected)
        }
    }
}

#[cfg(test)]
#[path = "render_tests.rs"]
mod tests;
