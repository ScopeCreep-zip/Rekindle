//! The DM video frame commands (E4.1, E4.3.8). The group-DM commands are
//! E2.6 slice G3's, over `rekindle-dm`.

use tauri::State;

use crate::state::SharedState;

/// W11.4 (P6.2) — send one encoded video frame to a 1:1 DM peer.
///
/// Mirrors `commands::community::video::send_video_frame` but routes
/// 1:1 over the peer's ratchet session (`operations::dm::send_to_peer`)
/// instead of mesh fan-out, and uses no community MEK (the Double Ratchet
/// already covers confidentiality + authenticity).
///
/// The frontend produces VP9 chunks via WebCodecs and base64-encodes
/// them for IPC; the backend chunks to the 4 KB transport budget, wraps each chunk in a
/// `DmVideoFragment` (0x0220) frame, and sends through the 1:1
/// transport. Returns the number of fragments dispatched.
#[tauri::command]
pub async fn send_dm_video_frame(
    peer_pubkey: String,
    request: SendDmVideoFrameRequest,
    state: State<'_, SharedState>,
) -> Result<u32, String> {
    crate::services::dm_runtime::send_dm_video_frame_inner(state.inner(), peer_pubkey, request)
        .await
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SendDmVideoFrameRequest {
    pub stream_id_hex: String,
    pub frame_seq: u32,
    pub keyframe: bool,
    /// Codec wire string ("vp9" | "vp8" | "h264").
    pub codec: String,
    pub timestamp: u32,
    pub encoded_payload_b64: String,
}

/// Phase 11 Tier 1 — register the per-peer `ipc::Channel` the DM video
/// panel listens on. Inbound reassembled frames for `peer_pubkey` are
/// pushed straight to this channel instead of the `dm-video-frame`
/// event. Re-registering replaces the previous handle.
#[tauri::command]
pub async fn register_dm_video_channel(
    peer_pubkey: String,
    on_frame: tauri::ipc::Channel<crate::video_channels::DmVideoFrameMsg>,
    state: State<'_, SharedState>,
) -> Result<(), String> {
    state.video_channels.register_dm(peer_pubkey, on_frame);
    Ok(())
}

/// Phase 11 Tier 1 — drop the per-peer DM video channel when the panel
/// unmounts so frames stop being forwarded.
#[tauri::command]
pub async fn unregister_dm_video_channel(
    peer_pubkey: String,
    state: State<'_, SharedState>,
) -> Result<(), String> {
    state.video_channels.unregister_dm(&peer_pubkey);
    Ok(())
}
