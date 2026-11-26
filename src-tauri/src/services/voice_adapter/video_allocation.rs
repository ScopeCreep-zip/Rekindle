//! The desktop's video encoders as the allocation follower drives them
//! (`rekindle_voice::transport::allocation_follower`, plan E4.3.3, moved
//! by E4.1): the native camera session directly, the webview encoder
//! through `VideoBitrateTarget` and `VideoKeyframeRequest`.

use std::sync::Arc;

use rekindle_types::media::Track;
use rekindle_voice::transport::allocation_follower::VideoEncoders;

use crate::channels::{CommunityEvent, VideoBitrateTargetEvent, VideoKeyframeRequestEvent};
use crate::state::AppState;

/// The current video encoder target, for an encoder starting mid-call.
#[must_use]
pub fn encoder_kbps(state: &AppState) -> u32 {
    let allocation = state
        .voice_engine
        .lock()
        .as_ref()
        .map(|h| *h.media.allocator().subscribe().borrow())
        .unwrap_or_default();
    rekindle_voice::transport::allocation_follower::encoder_kbps(allocation)
}

pub struct DesktopVideoEncoders {
    pub state: Arc<AppState>,
    pub app: tauri::AppHandle,
    pub community_id: String,
    pub channel_id: String,
}

impl VideoEncoders for DesktopVideoEncoders {
    fn set_native_kbps(&self, kbps: u32) {
        crate::services::native_video::set_target_kbps(&self.state, kbps);
    }

    fn send_webview_kbps(&self, kbps: u32) {
        crate::event_dispatch::emit_community(
            &self.app,
            CommunityEvent::VideoBitrateTarget(VideoBitrateTargetEvent {
                community_id: self.community_id.clone(),
                channel_id: self.channel_id.clone(),
                kbps,
            }),
        );
    }

    fn force_native_keyframes(&self) {
        crate::services::native_video::force_keyframes(&self.state);
    }

    fn request_webview_keyframe(&self, track: Track) {
        let Some(pseudonym) =
            crate::state_helpers::my_pseudonym_key(&self.state, &self.community_id)
        else {
            return;
        };
        // The stream id our community stream on `track` is derived under
        // (`video_sender.ts`).
        let stream_id =
            rekindle_video::derive_stream_id(&self.channel_id, &pseudonym, track.label());
        crate::event_dispatch::emit_community(
            &self.app,
            CommunityEvent::VideoKeyframeRequest(VideoKeyframeRequestEvent {
                community_id: self.community_id.clone(),
                sender_pseudonym: pseudonym,
                channel_id: self.channel_id.clone(),
                stream_id: hex::encode(stream_id),
            }),
        );
    }
}
