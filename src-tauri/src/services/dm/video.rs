//! Phase 13 — thin Tier-9 bridge between the DM video transport and
//! the rekindle-dm reassembly primitive.
//!
//! `DmVideoReassemblyState`, `AssembledFrame`, and `FRAGMENT_PAYLOAD_LIMIT`
//! moved to `rekindle-dm::video` (pure buffer ops, no `veilid_core` or
//! `AppState` deps). This file re-exports them at the prior path so
//! existing call sites in `state.rs` + `message_service.rs` compile
//! unchanged, and keeps `send_dm_video_frame` — the outbound
//! orchestration that fragments a frame and dispatches each chunk via
//! the ratchet-sealed 1:1 transport (`operations::dm::send_to_peer`, §6
//! 0x0220 until E4.3.8).

use std::sync::Arc;

use rekindle_codec::capnp_codec::direct::{DirectBody, DmVideoFragment};

use crate::state::AppState;

// Re-export the moved primitives so existing import paths still work.
pub use rekindle_dm::{AssembledFrame, DmVideoReassemblyState, FRAGMENT_PAYLOAD_LIMIT};

/// One outbound DM video frame, decoded from the Tauri command's
/// request shape. Bundled so the send fn stays under the argument
/// budget.
pub struct DmVideoFrameSend {
    pub stream_id: [u8; 16],
    pub frame_seq: u32,
    pub keyframe: bool,
    /// Codec wire string the frontend encoder produced this frame with.
    pub codec: String,
    pub timestamp: u32,
    pub encoded_payload: Vec<u8>,
}

/// Fragment a frame's encoded payload into ≤`FRAGMENT_PAYLOAD_LIMIT`
/// chunks and send each as a `DmVideoFragment` frame over the ratchet.
/// Returns the number of fragments sent.
pub async fn send_dm_video_frame(
    state: &Arc<AppState>,
    peer_pubkey: &str,
    frame: DmVideoFrameSend,
) -> Result<u32, String> {
    let DmVideoFrameSend {
        stream_id,
        frame_seq,
        keyframe,
        codec,
        timestamp,
        encoded_payload,
    } = frame;
    if encoded_payload.is_empty() {
        return Err("empty encoded payload".to_string());
    }
    // Hard error on unknown codec wire strings — same gate as the
    // community sender (`community_video_runtime`). Normalizing through
    // the typed enum also guarantees the on-wire tag is canonical.
    let codec = rekindle_types::video::Codec::from_wire_str(&codec)
        .ok_or_else(|| format!("unknown codec wire string: {codec}"))?
        .wire_str()
        .to_string();
    let chunks: Vec<&[u8]> = encoded_payload.chunks(FRAGMENT_PAYLOAD_LIMIT).collect();
    let fragment_count = u16::try_from(chunks.len())
        .map_err(|_| format!("frame too large to fragment ({} chunks)", chunks.len()))?;

    for (idx, chunk) in chunks.iter().enumerate() {
        let body = DirectBody::DmVideoFragment(DmVideoFragment {
            stream_id,
            frame_seq,
            fragment_index: u16::try_from(idx).expect("checked above"),
            fragment_count,
            keyframe,
            codec: codec.clone(),
            timestamp,
            chunk: chunk.to_vec(),
        });
        // Encrypted fail-closed: vulnerable users are protected from a
        // plaintext fallback that an attacker could trigger by
        // corrupting the session.
        rekindle_transport::operations::dm::send_to_peer(
            crate::state_helpers::friend_host(state)?.as_ref(),
            peer_pubkey,
            &body,
        )
        .await
        .map_err(|e| e.to_string())?;
    }
    Ok(u32::from(fragment_count))
}

/// Deliver `sender`'s reassembled frame to the DM video panel's per-peer
/// `ipc::Channel` (a no-op when no panel is open), after noting it in the
/// call's quality stats. DM video carries no parity: a completed frame is
/// never FEC-recovered (plan E4.3 Q0 post-FEC loss).
pub fn deliver_assembled_frame(state: &AppState, sender: &str, frame: AssembledFrame) {
    use base64::Engine as _;
    let stream_id_hex = hex::encode(frame.stream_id);
    if let Some(media) = crate::state_helpers::voice_media(state) {
        media
            .quality()
            .note_frame_completed(&stream_id_hex, sender, frame.frame_seq, false);
    }
    state.video_channels.send_dm(
        sender,
        crate::video_channels::DmVideoFrameMsg {
            peer_pubkey: sender.to_owned(),
            stream_id_hex,
            frame_seq: frame.frame_seq,
            keyframe: frame.keyframe,
            codec: frame.codec,
            timestamp: frame.timestamp,
            encoded_payload_b64: base64::engine::general_purpose::STANDARD.encode(&frame.data),
        },
    );
}
