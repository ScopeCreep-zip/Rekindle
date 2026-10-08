//! Community video follows the allocator (plan E4.3.3).
//!
//! The voice transport's allocator splits each route's estimate between
//! audio and video. The send loop applies the audio half to Opus; this
//! task applies the video half to whichever encoder is running — the
//! native camera session directly, the webview encoder through
//! `VideoBitrateTarget` — and turns a route's "keyframe wanted" (a new
//! route, or video dropped past the queue-time bound) into a keyframe
//! request for our own streams.

use std::sync::Arc;

use rekindle_voice::transport::allocation::{Allocation, Allocator, VIDEO_MIN_BPS};

use crate::channels::{CommunityEvent, VideoBitrateTargetEvent, VideoKeyframeRequestEvent};
use crate::state::AppState;

/// A webview encoder reconfigure forces a keyframe, so its target is
/// re-sent only when it moves more than this fraction.
const WEBVIEW_HYSTERESIS: f64 = 0.15;

/// The track labels our community streams are derived under
/// (`video_sender.ts`).
const TRACK_LABELS: [&str; 2] = ["camera", "screen"];

/// The encoder rate for `allocation`, at least video's minimum (before any
/// route has an estimate the allocation carries 0).
fn kbps_for(allocation: Allocation) -> u32 {
    allocation.video_bps.max(VIDEO_MIN_BPS) / 1_000
}

/// The current video encoder target, for an encoder starting mid-call.
#[must_use]
pub fn encoder_kbps(state: &AppState) -> u32 {
    let allocation = state
        .voice_engine
        .lock()
        .as_ref()
        .map(|h| *h.media.allocator().subscribe().borrow())
        .unwrap_or_default();
    kbps_for(allocation)
}

pub struct VideoAllocationFollower {
    pub state: Arc<AppState>,
    pub app: tauri::AppHandle,
    pub allocator: Arc<Allocator>,
    pub community_id: String,
    pub channel_id: String,
}

impl VideoAllocationFollower {
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
        let kbps = kbps_for(allocation);
        tracing::info!(
            target: "rekindle_video::allocation",
            community_id = %self.community_id,
            channel_id = %self.channel_id,
            audio_bps = allocation.audio_bps,
            video_bps = allocation.video_bps,
            encoder_kbps = kbps,
            "video bitrate allocated"
        );
        crate::services::native_video::set_target_kbps(&self.state, kbps);
        let moved = webview_kbps.is_none_or(|last| {
            (f64::from(kbps) - f64::from(last)).abs() / f64::from(last.max(1)) > WEBVIEW_HYSTERESIS
        });
        if moved {
            *webview_kbps = Some(kbps);
            crate::event_dispatch::emit_community(
                &self.app,
                CommunityEvent::VideoBitrateTarget(VideoBitrateTargetEvent {
                    community_id: self.community_id.clone(),
                    channel_id: self.channel_id.clone(),
                    kbps,
                }),
            );
        }
    }

    /// A keyframe on every stream we send: the native session directly,
    /// the webview's through the keyframe request a receiver would send
    /// for them (the sender acts only on the streams it owns).
    fn request_keyframe(&self) {
        crate::services::native_video::force_keyframes(&self.state);
        let Some(pseudonym) =
            crate::state_helpers::my_pseudonym_key(&self.state, &self.community_id)
        else {
            return;
        };
        for label in TRACK_LABELS {
            let stream_id = rekindle_video::derive_stream_id(&self.channel_id, &pseudonym, label);
            crate::event_dispatch::emit_community(
                &self.app,
                CommunityEvent::VideoKeyframeRequest(VideoKeyframeRequestEvent {
                    community_id: self.community_id.clone(),
                    sender_pseudonym: pseudonym.clone(),
                    channel_id: self.channel_id.clone(),
                    stream_id: hex::encode(stream_id),
                }),
            );
        }
    }
}
