//! Splitting one route's estimate between audio and video (plan E4.3.3,
//! ADR 0015).
//!
//! The phases follow libwebrtc's `BitrateAllocator`
//! (`call/bitrate_allocator.cc`): every stream gets its minimum if the
//! estimate allows, then the remainder is shared out up to each maximum.
//! The priority is Rekindle's: audio is filled first, within its range,
//! and always keeps its minimum (libwebrtc's `enforce_min_bitrate`, which
//! audio streams set); video takes what remains and is paused below its
//! minimum. A paused video resumes only once the remainder clears its
//! minimum by libwebrtc's `kToggleFactor` (10 %), so it does not flap at
//! the edge.
//!
//! The estimate is a rate on the wire, transport overhead included. Each
//! stream's cost on the wire is given by the caller: audio as its encoder
//! rate plus a fixed overhead (a fixed packet rate times a per-packet
//! cost), video as a share of encoder bytes per wire byte.

use crate::Bitrate;

/// libwebrtc `bitrate_allocator.cc` `kToggleFactor`.
const TOGGLE_FACTOR: f64 = 0.1;

/// An encoder's range.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Range {
    pub min: Bitrate,
    pub max: Bitrate,
}

/// Audio's encoder range and what it costs on the wire beyond its
/// encoder rate.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AudioCost {
    pub range: Range,
    pub overhead: Bitrate,
}

/// Video's encoder range and its encoder bytes per wire byte.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VideoCost {
    pub range: Range,
    pub share: f64,
}

/// One route's split, in encoder rates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Split {
    pub audio: Bitrate,
    /// `None` while video is paused on the route.
    pub video: Option<Bitrate>,
    /// What the route would carry unconstrained, for the estimator's
    /// probes: audio at its maximum, plus video at its maximum when video
    /// is being offered (str0m `set_bwe_desired_bitrate`).
    pub desired: Bitrate,
}

/// Split `estimate` between audio and video.
#[must_use]
pub fn split(
    estimate: Bitrate,
    audio: AudioCost,
    video: VideoCost,
    video_offered: bool,
    video_was_allowed: bool,
) -> Split {
    let estimate = estimate.as_f64();
    let overhead = audio.overhead.as_f64();
    let audio_max = audio.range.max.as_f64() + overhead;
    let audio_on_wire = estimate.clamp(audio.range.min.as_f64() + overhead, audio_max);

    let share = video.share.max(0.01);
    let room = (estimate - audio_on_wire).max(0.0) * share;
    let min = video.range.min.as_f64();
    let needed = if video_was_allowed {
        min
    } else {
        min * (1.0 + TOGGLE_FACTOR)
    };
    let video_rate = (room >= needed).then(|| Bitrate::from(room.min(video.range.max.as_f64())));

    let video_desired = if video_offered {
        video.range.max.as_f64() / share
    } else {
        0.0
    };
    Split {
        audio: Bitrate::from(audio_on_wire - overhead),
        video: video_rate,
        desired: Bitrate::from(audio_max + video_desired),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 50 packets a second at 1800 B of overhead each.
    const OVERHEAD: Bitrate = Bitrate::bps(720_000);

    fn audio() -> AudioCost {
        AudioCost {
            range: Range {
                min: Bitrate::kbps(24),
                max: Bitrate::kbps(64),
            },
            overhead: OVERHEAD,
        }
    }

    fn video() -> VideoCost {
        VideoCost {
            range: Range {
                min: Bitrate::kbps(100),
                max: Bitrate::kbps(600),
            },
            share: 0.5,
        }
    }

    fn at(bps: u64, was_allowed: bool) -> Split {
        split(Bitrate::bps(bps), audio(), video(), true, was_allowed)
    }

    #[test]
    fn audio_keeps_its_minimum_below_any_estimate() {
        let s = at(100_000, true);
        assert_eq!(s.audio, Bitrate::kbps(24));
        assert_eq!(s.video, None);
    }

    #[test]
    fn between_audio_min_and_max_audio_takes_it_all() {
        let s = at(720_000 + 44_000, true);
        assert_eq!(s.audio, Bitrate::kbps(44));
        assert_eq!(s.video, None);
    }

    #[test]
    fn audio_fills_first_then_video_takes_the_rest() {
        let s = at(784_000 + 400_000, true);
        assert_eq!(s.audio, Bitrate::kbps(64));
        assert_eq!(
            s.video,
            Some(Bitrate::kbps(200)),
            "400 kbps of wire at a 0.5 share"
        );
        assert_eq!(at(10_000_000, true).video, Some(Bitrate::kbps(600)));
    }

    #[test]
    fn paused_video_resumes_only_past_the_toggle_margin() {
        // 210 kbps of wire → 105 kbps of video: enough to keep, not resume.
        assert!(at(784_000 + 210_000, true).video.is_some());
        assert!(at(784_000 + 210_000, false).video.is_none());
        assert!(at(784_000 + 230_000, false).video.is_some());
    }

    #[test]
    fn desired_includes_video_only_when_offered() {
        let without = split(Bitrate::kbps(300), audio(), video(), false, true);
        let with = split(Bitrate::kbps(300), audio(), video(), true, true);
        assert_eq!(without.desired, Bitrate::bps(784_000));
        assert_eq!(with.desired, Bitrate::bps(784_000 + 1_200_000));
    }
}
