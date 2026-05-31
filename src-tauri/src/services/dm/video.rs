//! Phase 13 — thin Tier-9 bridge between the DM video transport and
//! the rekindle-dm reassembly primitive.
//!
//! `DmVideoReassemblyState`, `AssembledFrame`, and `FRAGMENT_PAYLOAD_LIMIT`
//! moved to `rekindle-dm::video` (pure buffer ops, no `veilid_core` or
//! `AppState` deps). This file re-exports them at the prior path so
//! existing call sites in `state.rs` + `message_service.rs` compile
//! unchanged, and keeps `send_dm_video_frame` — the outbound
//! orchestration that fragments a frame and dispatches each chunk via
//! the Signal-encrypted DM transport (`message_service::send_to_peer_encrypted`).

use std::sync::Arc;

use rekindle_protocol::messaging::envelope::MessagePayload;

use crate::db::DbPool;
use crate::services::message_service;
use crate::state::AppState;

// Re-export the moved primitives so existing import paths still work.
pub use rekindle_dm::{DmVideoReassemblyState, FRAGMENT_PAYLOAD_LIMIT};

/// Fragment a frame's encoded payload into ≤`FRAGMENT_PAYLOAD_LIMIT`
/// chunks and send each as a `MessagePayload::DmVideoFragment` via the
/// existing Signal-encrypted DM transport. Returns the number of
/// fragments sent.
pub async fn send_dm_video_frame(
    state: &Arc<AppState>,
    pool: &DbPool,
    peer_pubkey: &str,
    stream_id: [u8; 16],
    frame_seq: u32,
    keyframe: bool,
    timestamp: u32,
    encoded_payload: &[u8],
) -> Result<u32, String> {
    if encoded_payload.is_empty() {
        return Err("empty encoded payload".to_string());
    }
    let chunks: Vec<&[u8]> = encoded_payload.chunks(FRAGMENT_PAYLOAD_LIMIT).collect();
    let fragment_count = u16::try_from(chunks.len())
        .map_err(|_| format!("frame too large to fragment ({} chunks)", chunks.len()))?;

    for (idx, chunk) in chunks.iter().enumerate() {
        let payload = MessagePayload::DmVideoFragment {
            stream_id,
            frame_seq,
            fragment_index: u16::try_from(idx).expect("checked above"),
            fragment_count,
            keyframe,
            timestamp,
            chunk: chunk.to_vec(),
        };
        // Encrypted fail-closed: vulnerable users are protected from a
        // plaintext fallback that an attacker could trigger by
        // corrupting the Signal session.
        message_service::send_to_peer_encrypted(state, pool, peer_pubkey, &payload).await?;
    }
    Ok(u32::from(fragment_count))
}
