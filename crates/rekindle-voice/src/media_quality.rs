//! What the call looks and sounds like at this receiver (plan E4.3 Q0).
//!
//! One ledger per session, on the [`MediaRoster`](crate::transport::roster::MediaRoster):
//!
//! - per video stream: freezes and pauses as rendered (W3C webrtc-stats
//!   definitions, [`rekindle_media_stats::render`]) and post-FEC frame loss;
//! - per sender: lip sync, from the audio offsets the receive loop notes at
//!   playout and the video offsets the webview reports at render
//!   ([`rekindle_media_stats::sync`]).
//!
//! The receive loop logs a window every stats period and a summary when it
//! ends, next to the route summary (E4.3.0), so a call's run can be scored
//! against the requirements in `evidence/e4-3-call-quality-standards.md`.

use std::collections::HashMap;

use parking_lot::Mutex;
use rekindle_media_stats::{ReceiveCounters, RenderTracker, SyncTracker};

#[derive(Default)]
struct VideoStream {
    sender: String,
    render: RenderTracker,
    receive: ReceiveCounters,
}

#[derive(Default)]
struct Ledger {
    /// Keyed by stream id (hex).
    video: HashMap<String, VideoStream>,
    /// Keyed by sender key (hex): the voice packet's sender key, which is
    /// the video's sender pseudonym in a channel and the identity key in a
    /// DM call.
    sync: HashMap<String, SyncTracker>,
}

#[derive(Default)]
pub struct MediaQuality {
    ledger: Mutex<Ledger>,
}

/// `sender_ms` as the low 32 bits a video frame carries, widened back to
/// the full wall clock nearest `now_ms`.
#[must_use]
pub fn unwrap_wall_ms(wire_ms: u32, now_ms: u64) -> u64 {
    let behind = u32::try_from(now_ms & u64::from(u32::MAX))
        .unwrap_or_default()
        .wrapping_sub(wire_ms);
    now_ms.wrapping_sub(u64::from(behind))
}

fn offset_ms(local_ms: u64, sender_ms: u64) -> i64 {
    i64::try_from(local_ms).unwrap_or(i64::MAX) - i64::try_from(sender_ms).unwrap_or(i64::MAX)
}

impl MediaQuality {
    /// An audio frame from `sender` reaches the speaker at local wall
    /// `played_ms`; it was captured at sender wall `captured_ms`.
    pub fn note_audio_playout(&self, sender: &str, played_ms: u64, captured_ms: u64) {
        self.ledger
            .lock()
            .sync
            .entry(sender.to_string())
            .or_default()
            .note_audio(offset_ms(played_ms, captured_ms));
    }

    /// A video frame of `stream` from `sender` was rendered at local wall
    /// `rendered_ms`; it was captured at sender wall `captured_ms`.
    pub fn note_video_rendered(
        &self,
        stream: &str,
        sender: &str,
        rendered_ms: u64,
        captured_ms: u64,
    ) {
        let mut ledger = self.ledger.lock();
        let entry = ledger.video.entry(stream.to_string()).or_default();
        entry.sender = sender.to_string();
        entry.render.on_rendered(rendered_ms);
        ledger
            .sync
            .entry(sender.to_string())
            .or_default()
            .note_video(offset_ms(rendered_ms, captured_ms));
    }

    /// Reassembly completed frame `frame_seq` of `stream`.
    pub fn note_frame_completed(
        &self,
        stream: &str,
        sender: &str,
        frame_seq: u32,
        recovered_by_fec: bool,
    ) {
        let mut ledger = self.ledger.lock();
        let entry = ledger.video.entry(stream.to_string()).or_default();
        entry.sender = sender.to_string();
        entry.receive.on_completed(frame_seq, recovered_by_fec);
    }

    /// Take a lip-sync sample per sender and log every stream and sender
    /// so far. `ended` marks the session's final summary.
    pub fn log(&self, ended: bool) {
        let mut ledger = self.ledger.lock();
        for (sender, sync) in &mut ledger.sync {
            let skew = sync.sample();
            let s = sync.summary();
            if s.samples == 0 {
                continue;
            }
            tracing::info!(
                target: "rekindle_media::quality",
                sender = %sender,
                ended,
                skew_ms = ?skew,
                skew_p5_ms = s.skew_p5_ms,
                skew_p50_ms = s.skew_p50_ms,
                skew_p95_ms = s.skew_p95_ms,
                samples = s.samples,
                outside_p1305 = s.outside_bounds,
                "media quality: lip sync (video − audio; + = audio ahead)"
            );
        }
        for (stream, video) in &ledger.video {
            let r = video.render.summary();
            let c = video.receive;
            tracing::info!(
                target: "rekindle_media::quality",
                stream = %stream,
                sender = %video.sender,
                ended,
                frames_rendered = r.frames_rendered,
                fps = r.fps,
                harmonic_fps = r.harmonic_fps,
                freeze_count = r.freeze_count,
                total_freezes_ms = r.total_freezes_ms,
                pause_count = r.pause_count,
                total_pauses_ms = r.total_pauses_ms,
                frames_completed = c.completed,
                frames_recovered_by_fec = c.recovered_by_fec,
                frames_lost_post_fec = c.lost,
                post_fec_loss = c.loss_fraction(),
                "media quality: video"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unwrap_recovers_the_wall_clock() {
        let now = 1_791_442_864_000_u64;
        let sent = now - 250;
        let wire = u32::try_from(sent & u64::from(u32::MAX)).unwrap();
        assert_eq!(unwrap_wall_ms(wire, now), sent);
    }

    #[test]
    fn unwrap_crosses_a_wrap() {
        let now = (5_u64 << 32) + 100;
        let sent = now - 300;
        let wire = u32::try_from(sent & u64::from(u32::MAX)).unwrap();
        assert_eq!(unwrap_wall_ms(wire, now), sent);
    }

    #[test]
    fn audio_and_video_from_one_sender_give_its_skew() {
        let q = MediaQuality::default();
        for i in 0..10 {
            q.note_audio_playout("s", 10_000 + i * 20, 9_880 + i * 20);
            q.note_video_rendered("v", "s", 10_000 + i * 33, 9_840 + i * 33);
        }
        // Audio plays 120 ms after capture, video renders 160 ms after.
        let mut ledger = q.ledger.lock();
        assert_eq!(ledger.sync.get_mut("s").unwrap().sample(), Some(40));
    }
}
