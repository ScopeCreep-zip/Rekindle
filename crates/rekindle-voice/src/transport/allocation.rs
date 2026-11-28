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
#[path = "allocation_tests.rs"]
mod tests;
