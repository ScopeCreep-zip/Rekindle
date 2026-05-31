//! Phase 23.C — DM-handler Tauri-runtime orchestration lifted from
//! `commands/dm.rs`. Hosts `send_dm_video_frame_inner` — hex + base64
//! decoding wrapped around the crate-side video send.

use crate::db::DbPool;
use crate::services::dm;
use crate::state::SharedState;

pub async fn send_dm_video_frame_inner(
    state: &SharedState,
    pool: &DbPool,
    peer_pubkey: String,
    request: crate::commands::dm::SendDmVideoFrameRequest,
) -> Result<u32, String> {
    use base64::Engine as _;
    let stream_id_bytes =
        hex::decode(&request.stream_id_hex).map_err(|e| format!("invalid stream_id hex: {e}"))?;
    let stream_id: [u8; 16] = stream_id_bytes
        .as_slice()
        .try_into()
        .map_err(|_| "stream_id must be 16 bytes".to_string())?;
    let payload = base64::engine::general_purpose::STANDARD
        .decode(request.encoded_payload_b64.as_bytes())
        .map_err(|e| format!("invalid base64 payload: {e}"))?;
    dm::video::send_dm_video_frame(
        state,
        pool,
        &peer_pubkey,
        stream_id,
        request.frame_seq,
        request.keyframe,
        request.timestamp,
        &payload,
    )
    .await
}
