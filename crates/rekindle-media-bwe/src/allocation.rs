//! Splitting one route's estimate between audio and video (plan E4.3.3,
//! ADR 0015).
//!
//! The phases follow libwebrtc's `BitrateAllocator`
//! (`call/bitrate_allocator.cc`): every stream gets its minimum if the
//! estimate allows, then the remainder is shared out up to each maximum.
//! The priority is Rekindle's: audio is filled first, within its range;
//! video takes what remains, within its range. Both always keep their
//! minimum (libwebrtc's `enforce_min_bitrate`): audio streams set it, and so
//! does camera video, whose `suspend_below_min_bitrate` defaults to false
//! (`call/video_send_stream.h`, `video/video_send_stream_impl.cc`). Video
//! sent at its minimum is what lets the estimator find room for more; a
//! suspended stream sends nothing to measure, and with no periodic probing
//! it would stay suspended. Overload is bounded by the pacer's queue time.
//!
//! The estimate is a rate on the wire, transport overhead included. Each
//! stream's cost on the wire is given by the caller: audio as its encoder
//! rate plus a fixed overhead (a fixed packet rate times a per-packet
//! cost), video as a share of encoder bytes per wire byte.

use crate::Bitrate;

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
    pub video: Bitrate,
    /// What the route would carry unconstrained, for the estimator's
    /// probes: audio at its maximum, plus video at its maximum when video
    /// is being offered (str0m `set_bwe_desired_bitrate`).
    pub desired: Bitrate,
}

/// Split `estimate` between audio and video.
#[must_use]
pub fn split(estimate: Bitrate, audio: AudioCost, video: VideoCost, video_offered: bool) -> Split {
    let estimate = estimate.as_f64();
    let overhead = audio.overhead.as_f64();
    let audio_max = audio.range.max.as_f64() + overhead;
    let audio_on_wire = estimate.clamp(audio.range.min.as_f64() + overhead, audio_max);

    let share = video.share.max(0.01);
    let room = (estimate - audio_on_wire).max(0.0) * share;
    let video_rate = room.clamp(video.range.min.as_f64(), video.range.max.as_f64());

    let video_desired = if video_offered {
        video.range.max.as_f64() / share
    } else {
        0.0
    };
    Split {
        audio: Bitrate::from(audio_on_wire - overhead),
        video: Bitrate::from(video_rate),
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

    fn at(bps: u64) -> Split {
        split(Bitrate::bps(bps), audio(), video(), true)
    }

    #[test]
    fn audio_keeps_its_minimum_below_any_estimate() {
        let s = at(100_000);
        assert_eq!(s.audio, Bitrate::kbps(24));
        assert_eq!(s.video, Bitrate::kbps(100), "video keeps its minimum too");
    }

    #[test]
    fn between_audio_min_and_max_audio_takes_it_all() {
        let s = at(720_000 + 44_000);
        assert_eq!(s.audio, Bitrate::kbps(44));
        assert_eq!(s.video, Bitrate::kbps(100));
    }

    #[test]
    fn audio_fills_first_then_video_takes_the_rest() {
        let s = at(784_000 + 400_000);
        assert_eq!(s.audio, Bitrate::kbps(64));
        assert_eq!(
            s.video,
            Bitrate::kbps(200),
            "400 kbps of wire at a 0.5 share"
        );
        assert_eq!(at(10_000_000).video, Bitrate::kbps(600));
    }

    #[test]
    fn desired_includes_video_only_when_offered() {
        let without = split(Bitrate::kbps(300), audio(), video(), false);
        let with = split(Bitrate::kbps(300), audio(), video(), true);
        assert_eq!(without.desired, Bitrate::bps(784_000));
        assert_eq!(with.desired, Bitrate::bps(784_000 + 1_200_000));
    }
}
