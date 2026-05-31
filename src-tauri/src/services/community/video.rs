//! Architecture §10.6 community video / screen-share — thin Tier-9 facade.
//!
//! Phase 16 — all orchestration logic moved into `rekindle-video` +
//! `services::video_adapter`. This module now only re-exports a couple
//! of types for caller compat and delegates the two public entry points
//! (send / receive dispatch) to the adapter.

use std::sync::Arc;

use crate::state::AppState;

/// Re-export so existing call sites (`commands::community::video::send_video_frame`,
/// future callers) keep their import paths working.
pub use rekindle_video::VideoFrameSend;

/// Send a community video frame. Body lives in
/// `rekindle_video::send_video_frame` parameterised over `VideoDeps`.
pub fn send_video_frame(
    state: &crate::state::SharedState,
    community_id: &str,
    channel_id: &str,
    request: &VideoFrameSend,
) -> Result<u32, String> {
    crate::services::video_adapter::send_video_frame(state, community_id, channel_id, request)
}

/// Dispatch entry point — routed from `services/veilid/control_moderation`
/// when any video-flavoured `ControlPayload` arrives. Body lives in
/// `rekindle_video::handle_video_payload` parameterised over `VideoDeps`.
pub fn handle_video_payload(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    community_id: &str,
    sender_pseudonym: &str,
    payload: rekindle_protocol::dht::community::envelope::ControlPayload,
) {
    crate::services::video_adapter::handle_video_payload(
        app_handle,
        state,
        community_id,
        sender_pseudonym,
        payload,
    );
}
