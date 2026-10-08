//! A route's state for the `media route` log line.

use std::time::Instant;

use super::RouteController;

/// A route's state, for the log line.
#[derive(Debug, Clone, Copy)]
pub struct RouteStats {
    pub estimate_bps: u64,
    pub overusing: bool,
    pub video_queue_ms: u64,
    pub dropped_video: u64,
    pub sent: u64,
    /// Since the last stats: feedback reports applied, and the datagrams
    /// they reported received and lost (what the loss controller sees).
    pub feedback_reports: u64,
    pub reported_received: u64,
    pub reported_lost: u64,
    /// What the estimator based the estimate on (plan E4.3 T2
    /// diagnostics): acked rate, delay and loss estimates, loss state,
    /// RTT backoff cuts and probes applied.
    pub bwe: rekindle_media_bwe::BweDiagnostics,
    /// Voice frames per message (plan E4.3 T3).
    pub voice_frames: usize,
}

impl RouteController {
    /// For the periodic log line.
    pub fn stats(&mut self, now: Instant) -> RouteStats {
        let first = self.video.snapshot(now).first_unsent;
        let (feedback_reports, reported_received, reported_lost) =
            std::mem::take(&mut self.window_feedback);
        RouteStats {
            feedback_reports,
            reported_received,
            reported_lost,
            estimate_bps: self.estimate().as_u64(),
            overusing: self.bwe.is_overusing(),
            video_queue_ms: first.map_or(0, |t| {
                u64::try_from(now.saturating_duration_since(t).as_millis()).unwrap_or(u64::MAX)
            }),
            dropped_video: self.dropped_video,
            sent: self.next_seq,
            bwe: self.bwe.diagnostics(),
            voice_frames: self.voice_frames(),
        }
    }
}
