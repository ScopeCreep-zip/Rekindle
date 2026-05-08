//! W11.4 — DM video transport (1:1 video calls over the existing
//! Signal-encrypted DM transport).
//!
//! Mirrors the shape of `services::community::video` but routes 1:1
//! instead of mesh and inherits Signal Double Ratchet encryption from
//! `send_envelope_to_peer` instead of layering MEK on top. Frame
//! fragmentation matches community video's 30 KB chunk budget — the
//! Signal layer wraps each fragment as the inner plaintext of a
//! `MessagePayload::DmVideoFragment` so each on-wire envelope stays
//! within Veilid's 32 KB `MAX_APP_MESSAGE_MESSAGE_LEN`.
//!
//! Reassembly state is per-(peer_pubkey, stream_id, frame_seq) and is
//! pruned on a stale-frame TTL so a peer who drops mid-frame can't
//! pin memory indefinitely.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use rekindle_protocol::messaging::envelope::MessagePayload;

use crate::db::DbPool;
use crate::services::message_service;
use crate::state::AppState;

/// Per-fragment maximum chunk size. Leaves headroom under Veilid's
/// 32 KB app_message limit for Signal Double Ratchet overhead +
/// envelope framing. Mirrors the community video budget so the
/// frontend's keyframe-fragmentation logic stays unchanged.
pub const FRAGMENT_PAYLOAD_LIMIT: usize = 28 * 1024;

/// Reassembly buffers older than this are dropped during the next
/// `record_fragment` call. 5 s comfortably exceeds a stalled keyframe's
/// expected delivery window at 480p VP9 / 15 fps.
const REASSEMBLY_TTL: Duration = Duration::from_secs(5);

/// Soft cap on the number of in-flight frames per peer. Above this we
/// drop the oldest in-flight frame to bound memory under
/// adversarial-sender pressure.
const MAX_FRAMES_PER_PEER: usize = 64;

#[derive(Default)]
pub struct DmVideoReassemblyState {
    inner: Mutex<HashMap<String, PeerReassembly>>,
}

#[derive(Default)]
struct PeerReassembly {
    /// (stream_id, frame_seq) → partial frame.
    frames: HashMap<([u8; 16], u32), PartialFrame>,
}

struct PartialFrame {
    fragment_count: u16,
    keyframe: bool,
    timestamp: u32,
    /// Indexed by `fragment_index`; `None` until that fragment lands.
    chunks: Vec<Option<Vec<u8>>>,
    received_at: Instant,
}

impl DmVideoReassemblyState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record one inbound DM video fragment. Returns `Some(frame)` when
    /// the last missing fragment for this `(peer, stream, frame)` lands;
    /// the frame is removed from state on return so the caller hands it
    /// off to the decoder exactly once.
    pub fn record_fragment(
        &self,
        peer_pubkey: &str,
        stream_id: [u8; 16],
        frame_seq: u32,
        fragment_index: u16,
        fragment_count: u16,
        keyframe: bool,
        timestamp: u32,
        chunk: Vec<u8>,
    ) -> Option<AssembledFrame> {
        if fragment_count == 0 || fragment_index >= fragment_count {
            return None;
        }
        let mut state = self.inner.lock();
        let peer = state.entry(peer_pubkey.to_string()).or_default();

        // Prune frames that have stalled past TTL or are over the
        // per-peer soft cap (drop oldest by `received_at`).
        prune_stale(peer);

        let key = (stream_id, frame_seq);
        let entry = peer.frames.entry(key).or_insert_with(|| PartialFrame {
            fragment_count,
            keyframe,
            timestamp,
            chunks: vec![None; fragment_count as usize],
            received_at: Instant::now(),
        });
        // Defensive: a sender shouldn't change fragment_count mid-frame.
        // If they do, treat it as a fresh frame.
        if entry.fragment_count != fragment_count {
            *entry = PartialFrame {
                fragment_count,
                keyframe,
                timestamp,
                chunks: vec![None; fragment_count as usize],
                received_at: Instant::now(),
            };
        }
        entry.chunks[fragment_index as usize] = Some(chunk);
        entry.received_at = Instant::now();

        // Complete?
        if entry.chunks.iter().all(Option::is_some) {
            let assembled = peer.frames.remove(&key)?;
            let mut data = Vec::new();
            for chunk in assembled.chunks.into_iter().flatten() {
                data.extend(chunk);
            }
            return Some(AssembledFrame {
                stream_id,
                frame_seq,
                keyframe: assembled.keyframe,
                timestamp: assembled.timestamp,
                data,
            });
        }
        None
    }

    /// Drop all reassembly state for a peer (e.g. on call-end / hangup).
    pub fn forget_peer(&self, peer_pubkey: &str) {
        self.inner.lock().remove(peer_pubkey);
    }
}

fn prune_stale(peer: &mut PeerReassembly) {
    let now = Instant::now();
    peer.frames
        .retain(|_, p| now.saturating_duration_since(p.received_at) < REASSEMBLY_TTL);
    if peer.frames.len() > MAX_FRAMES_PER_PEER {
        let mut entries: Vec<_> = peer
            .frames
            .iter()
            .map(|(k, v)| (*k, v.received_at))
            .collect();
        entries.sort_by_key(|(_, ts)| *ts);
        let drop_count = peer.frames.len() - MAX_FRAMES_PER_PEER;
        for (key, _) in entries.into_iter().take(drop_count) {
            peer.frames.remove(&key);
        }
    }
}

pub struct AssembledFrame {
    pub stream_id: [u8; 16],
    pub frame_seq: u32,
    pub keyframe: bool,
    pub timestamp: u32,
    pub data: Vec<u8>,
}

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
