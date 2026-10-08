//! The session's allocation over its routes (plan E4.3.3).
//!
//! Each route's estimate is split by `rekindle_media_bwe::allocation`:
//! audio first within Opus's 24–64 kbps, video the rest between 100 and
//! 600 kbps, each keeping its minimum (`evidence/e4-3-bandwidth-owner-design.md` #6). This module
//! supplies what the split needs from the voice side — what each media
//! kind costs on a Veilid route — and keeps the session's targets.
//!
//! The estimate is a rate on the wire, Veilid's per-message cost included
//! (`egress::ROUTE_OVERHEAD_BYTES`). Voice runs at a fixed 50 packets a
//! second, so its wire cost is its Opus rate plus 50 times its measured
//! per-packet overhead; video's is its measured share of encoder bytes
//! per wire byte.
//!
//! A mesh sender has one encoder per media kind for all peers (#8), so it
//! encodes at the lowest allocation over the routes.

use std::collections::HashMap;

use parking_lot::Mutex;
use rekindle_media_bwe::allocation::{split, AudioCost, Range, VideoCost};
use rekindle_media_bwe::Bitrate;
use tokio::sync::{watch, Notify};

use super::egress::{MediaShare, ROUTE_OVERHEAD_BYTES};
use crate::codec::{DEFAULT_BITRATE_BPS, MIN_BITRATE_BPS};
use crate::media_frame::SEQUENCED_HEADER_LEN;

/// Voice packets a second: one 20 ms Opus frame each.
const AUDIO_PACKETS_PER_S: f64 = 50.0;
/// Video encoder range.
pub const VIDEO_MIN_BPS: u32 = 100_000;
pub const VIDEO_MAX_BPS: u32 = 600_000;
/// Video encoder bytes per wire byte before a route has measured any: a
/// full 4 KiB fragment with one parity fragment per four
/// (`rekindle-video` `PARITY_RATIO_DENOM`), each carrying the route's
/// overhead, about 4096 / (5120 + 1.25 × 1660).
const VIDEO_START_SHARE: f64 = 0.57;

/// One route's split, in encoder bits per second.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RouteAllocation {
    /// The route's estimate on the wire.
    pub estimate_bps: u64,
    pub audio_bps: u32,
    pub video_bps: u32,
    /// What the route would carry unconstrained, bits per second on the
    /// wire, for the estimator's probes.
    pub desired_on_wire: u64,
}

fn opus_bps(bps: i32) -> Bitrate {
    Bitrate::bps(u64::try_from(bps).unwrap_or_default())
}

/// Voice frames (20 ms each) per message the batching may choose: 20, 40,
/// 60 or 120 ms, libwebrtc's Opus frame lengths with 120 ms support
/// (`audio_encoder_opus.cc`).
pub const VOICE_FRAMES_PER_MESSAGE: [usize; 4] = [1, 2, 3, 6];

/// The audio payload that must remain once message overhead is paid
/// (libwebrtc `AdaptivePtimeConfig::min_payload_bitrate`, 16 kbps).
const MIN_AUDIO_PAYLOAD_BPS: f64 = 16_000.0;

/// How many 20 ms voice frames go in one message on a route: libwebrtc's
/// `FrameLengthControllerV2` rule (`frame_length_controller_v2.cc:59-71`),
/// the shortest frame length whose per-message overhead still leaves
/// [`MIN_AUDIO_PAYLOAD_BPS`] of the bandwidth audio has, or the longest
/// when none does. 3GPP calls it frame aggregation, the step after a
/// lower codec rate (TS 26.114 Annex C).
///
/// The bandwidth audio has is the route estimate less what video's minimum
/// costs on the wire (`use_slow_adaptation`: libwebrtc then compares the
/// uplink bandwidth). `overhead_bytes` is what one message costs beyond its
/// payload (Veilid's route plus our framing).
#[must_use]
pub fn voice_frames_per_message(
    estimate: Bitrate,
    video_share: Option<f64>,
    overhead_bytes: f64,
) -> usize {
    let video_min_on_wire =
        f64::from(VIDEO_MIN_BPS) / video_share.unwrap_or(VIDEO_START_SHARE).max(0.01);
    let audio_budget = estimate.as_f64() - video_min_on_wire;
    VOICE_FRAMES_PER_MESSAGE
        .iter()
        .copied()
        .find(|&n| {
            let message_ms = 20.0 * f64::from(u32::try_from(n).unwrap_or(1));
            audio_budget - overhead_bytes * 8.0 * 1_000.0 / message_ms > MIN_AUDIO_PAYLOAD_BPS
        })
        .unwrap_or(VOICE_FRAMES_PER_MESSAGE[VOICE_FRAMES_PER_MESSAGE.len() - 1])
}

/// Split `estimate` for one route, given what it measured of each media
/// kind's cost.
#[must_use]
pub fn allocate(estimate: Bitrate, share: MediaShare, video_offered: bool) -> RouteAllocation {
    // Until voice has been sent the overhead is at least the route's and
    // our sequence header.
    let audio_overhead = share.audio_overhead_bytes.unwrap_or_else(|| {
        f64::from(u32::try_from(ROUTE_OVERHEAD_BYTES + SEQUENCED_HEADER_LEN).unwrap_or(u32::MAX))
    });
    let audio = AudioCost {
        range: Range {
            min: opus_bps(MIN_BITRATE_BPS),
            max: opus_bps(DEFAULT_BITRATE_BPS),
        },
        overhead: Bitrate::from(AUDIO_PACKETS_PER_S * 8.0 * audio_overhead),
    };
    let video = VideoCost {
        range: Range {
            min: Bitrate::bps(u64::from(VIDEO_MIN_BPS)),
            max: Bitrate::bps(u64::from(VIDEO_MAX_BPS)),
        },
        share: share.video_share.unwrap_or(VIDEO_START_SHARE),
    };
    let s = split(estimate, audio, video, video_offered);
    RouteAllocation {
        estimate_bps: estimate.as_u64(),
        audio_bps: to_u32(s.audio),
        video_bps: to_u32(s.video),
        desired_on_wire: s.desired.as_u64(),
    }
}

fn to_u32(b: Bitrate) -> u32 {
    u32::try_from(b.as_u64()).unwrap_or(u32::MAX)
}

/// The encoder targets for the session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Allocation {
    pub audio_bps: u32,
    /// 0 until a route has an estimate.
    pub video_bps: u32,
}

impl Default for Allocation {
    fn default() -> Self {
        Self {
            audio_bps: u32::try_from(DEFAULT_BITRATE_BPS).unwrap_or(64_000),
            video_bps: 0,
        }
    }
}

/// The session's allocator: the latest split per route, and the encoder
/// targets over all of them.
pub struct Allocator {
    routes: Mutex<HashMap<String, RouteAllocation>>,
    targets: watch::Sender<Allocation>,
    keyframe: Notify,
}

impl Default for Allocator {
    fn default() -> Self {
        Self::new()
    }
}

impl Allocator {
    #[must_use]
    pub fn new() -> Self {
        Self {
            routes: Mutex::new(HashMap::new()),
            targets: watch::Sender::new(Allocation::default()),
            keyframe: Notify::new(),
        }
    }

    /// Record `peer`'s split and republish the encoder targets if they moved.
    pub fn note(&self, peer: &str, allocation: RouteAllocation) {
        let mut routes = self.routes.lock();
        routes.insert(peer.to_string(), allocation);
        self.publish(&routes);
    }

    /// `peer` left the roster.
    pub fn forget(&self, peer: &str) {
        let mut routes = self.routes.lock();
        if routes.remove(peer).is_some() {
            self.publish(&routes);
        }
    }

    /// The last split for `peer`.
    #[must_use]
    pub fn route(&self, peer: &str) -> Option<RouteAllocation> {
        self.routes.lock().get(peer).copied()
    }

    fn publish(&self, routes: &HashMap<String, RouteAllocation>) {
        let audio_bps = routes.values().map(|r| r.audio_bps).min();
        let video_bps = routes.values().map(|r| r.video_bps).min().unwrap_or(0);
        let next = Allocation {
            audio_bps: audio_bps.unwrap_or(Allocation::default().audio_bps),
            video_bps,
        };
        self.targets.send_if_modified(|current| {
            let changed = *current != next;
            *current = next;
            changed
        });
    }

    /// Encoder targets, as they change.
    #[must_use]
    pub fn subscribe(&self) -> watch::Receiver<Allocation> {
        self.targets.subscribe()
    }

    /// A route dropped video or resumed it: the local encoder should send
    /// a keyframe.
    pub fn request_keyframe(&self) {
        self.keyframe.notify_one();
    }

    /// Wait for the next keyframe request.
    pub async fn keyframe_requested(&self) {
        self.keyframe.notified().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn measured_costs_convert_to_encoder_rates() {
        let share = MediaShare {
            audio_overhead_bytes: Some(1_800.0),
            video_share: Some(0.5),
        };
        // Audio at 64 kbps costs 64k + 50 × 8 × 1800 = 784 kbps of wire.
        let a = allocate(Bitrate::bps(784_000 + 400_000), share, true);
        assert_eq!((a.audio_bps, a.video_bps), (64_000, 200_000));
        assert_eq!(a.estimate_bps, 1_184_000);
        let starved = allocate(Bitrate::kbps(300), share, true);
        assert_eq!((starved.audio_bps, starved.video_bps), (24_000, 100_000));
    }

    #[test]
    fn voice_batches_grow_as_the_route_narrows() {
        let h = 1_665.0;
        // Video's minimum at a 0.57 share costs ~175 kbps on the wire.
        // 20 ms needs 666 + 16 kbps for audio, 40 ms 333 + 16, 60 ms 222
        // + 16, 120 ms 111 + 16.
        assert_eq!(
            voice_frames_per_message(Bitrate::kbps(1_500), Some(0.57), h),
            1
        );
        assert_eq!(
            voice_frames_per_message(Bitrate::kbps(600), Some(0.57), h),
            2
        );
        assert_eq!(
            voice_frames_per_message(Bitrate::kbps(500), Some(0.57), h),
            3
        );
        assert_eq!(
            voice_frames_per_message(Bitrate::kbps(320), Some(0.57), h),
            6
        );
        assert_eq!(
            voice_frames_per_message(Bitrate::kbps(100), Some(0.57), h),
            6,
            "the longest when none fits"
        );
    }

    #[test]
    fn the_session_encodes_at_the_lowest_route() {
        let alloc = Allocator::new();
        let rx = alloc.subscribe();
        alloc.note(
            "a",
            RouteAllocation {
                estimate_bps: 0,
                audio_bps: 64_000,
                video_bps: 500_000,
                desired_on_wire: 0,
            },
        );
        alloc.note(
            "b",
            RouteAllocation {
                estimate_bps: 0,
                audio_bps: 40_000,
                video_bps: 200_000,
                desired_on_wire: 0,
            },
        );
        assert_eq!(
            *rx.borrow(),
            Allocation {
                audio_bps: 40_000,
                video_bps: 200_000
            },
            "one encoder per kind: the lowest route sets both"
        );
        alloc.forget("b");
        assert_eq!(rx.borrow().audio_bps, 64_000);
    }
}
