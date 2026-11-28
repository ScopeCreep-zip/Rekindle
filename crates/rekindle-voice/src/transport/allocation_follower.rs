//! Community video follows the allocator (plan E4.3.3; moved from
//! `src-tauri/src/services/voice_adapter/video_allocation.rs` by E4.1).
//!
//! The voice transport's allocator splits each route's estimate between
//! audio and video. The send loop applies the audio half to Opus; this
//! task applies the video half to whichever encoder is running — the
//! native camera session directly, the webview encoder through its
//! bitrate target — and turns a route's "keyframe wanted" (a new route,
//! or video dropped past the queue-time bound) into a keyframe request
//! for our own streams.

use std::sync::Arc;

use rekindle_types::media::Track;

use super::allocation::{Allocation, Allocator, VIDEO_MIN_BPS};

/// A webview encoder reconfigure forces a keyframe, so its target is
/// re-sent only when it moves more than this fraction.
const WEBVIEW_HYSTERESIS: f64 = 0.15;

/// The video encoder rate for `allocation`, at least video's minimum
/// (before any route has an estimate the allocation carries 0); also the
/// start rate for an encoder starting mid-call.
#[must_use]
pub fn encoder_kbps(allocation: Allocation) -> u32 {
    allocation.video_bps.max(VIDEO_MIN_BPS) / 1_000
}

/// Whether a webview target of `kbps` is worth a reconfigure after `last`.
fn webview_moved(last: Option<u32>, kbps: u32) -> bool {
    last.is_none_or(|last| {
        (f64::from(kbps) - f64::from(last)).abs() / f64::from(last.max(1)) > WEBVIEW_HYSTERESIS
    })
}

/// The host's video encoders, as the follower drives them.
pub trait VideoEncoders: Send + Sync {
    /// Retarget the native camera session.
    fn set_native_kbps(&self, kbps: u32);
    /// Send the webview encoder its new target.
    fn send_webview_kbps(&self, kbps: u32);
    /// Force a keyframe on the native session.
    fn force_native_keyframes(&self);
    /// Ask the webview for a keyframe on our `track` stream, as a receiver's
    /// keyframe request would (the sender acts only on streams it owns).
    fn request_webview_keyframe(&self, track: Track);
}

pub struct VideoAllocationFollower<E: VideoEncoders> {
    pub encoders: E,
    pub allocator: Arc<Allocator>,
    pub community_id: String,
    pub channel_id: String,
}

impl<E: VideoEncoders> VideoAllocationFollower<E> {
    pub async fn run(self, stop: tokio_util::sync::CancellationToken) {
        let mut targets = self.allocator.subscribe();
        let mut webview_kbps: Option<u32> = None;
        loop {
            tokio::select! {
                biased;
                () = stop.cancelled() => return,
                changed = targets.changed() => {
                    if changed.is_err() {
                        return;
                    }
                    let allocation = *targets.borrow_and_update();
                    self.apply(allocation, &mut webview_kbps);
                }
                () = self.allocator.keyframe_requested() => self.request_keyframe(),
            }
        }
    }

    fn apply(&self, allocation: Allocation, webview_kbps: &mut Option<u32>) {
        let kbps = encoder_kbps(allocation);
        tracing::info!(
            target: "rekindle_video::allocation",
            community_id = %self.community_id,
            channel_id = %self.channel_id,
            audio_bps = allocation.audio_bps,
            video_bps = allocation.video_bps,
            encoder_kbps = kbps,
            "video bitrate allocated"
        );
        self.encoders.set_native_kbps(kbps);
        if webview_moved(*webview_kbps, kbps) {
            *webview_kbps = Some(kbps);
            self.encoders.send_webview_kbps(kbps);
        }
    }

    /// A keyframe on every stream we send.
    fn request_keyframe(&self) {
        self.encoders.force_native_keyframes();
        for track in Track::ALL {
            self.encoders.request_webview_keyframe(track);
        }
    }
}

#[cfg(test)]
#[path = "allocation_follower_tests.rs"]
mod tests;
