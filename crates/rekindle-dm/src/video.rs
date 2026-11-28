//! Phase 13 — DM video frame reassembly.
//!
//! Mirrors the shape of community video but routes 1:1 instead of mesh
//! and inherits Signal Double Ratchet encryption from the DM transport
//! layer (handled in src-tauri). Frame fragmentation uses a 28 KB
//! chunk budget so each on-wire envelope stays within Veilid's 32 KB
//! `MAX_APP_MESSAGE_MESSAGE_LEN` after Signal Double Ratchet overhead
//! + envelope framing.
//!
//! Reassembly state is per-(peer_pubkey, stream_id, frame_seq) and is
//! pruned on a stale-frame TTL so a peer who drops mid-frame can't
//! pin memory indefinitely. Pure logic; no `veilid-core` or Tauri
//! dependencies — the src-tauri shell holds an instance on `AppState`
//! and dispatches fragments to it from `message_service`.

use std::collections::HashMap;
use std::time::Duration;
use tokio::time::Instant;

use parking_lot::Mutex;

/// Per-fragment maximum chunk size, sized for loss granularity rather
/// than the 32 KB `app_message` ceiling: Veilid segments every
/// envelope on a UDP hop into 1,272-byte fire-and-forget datagrams
/// with all-or-nothing reassembly, so a 28 KiB fragment rode as ~23
/// datagrams per hop and one lost datagram killed the whole fragment.
/// 4 KiB ≈ 4-5 datagrams/hop. Mirrors the community video budget
/// (rekindle-video `fragment.rs`) so the frontend's fragmentation
/// logic stays unchanged.
pub const FRAGMENT_PAYLOAD_LIMIT: usize = 4 * 1024;

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
    codec: String,
    timestamp: u32,
    /// Indexed by `fragment_index`; `None` until that fragment lands.
    chunks: Vec<Option<Vec<u8>>>,
    received_at: Instant,
}

/// One inbound DM video fragment: the 0x0220 body (§6), the one
/// definition of its fields.
pub use rekindle_codec::capnp_codec::direct::DmVideoFragment;

impl DmVideoReassemblyState {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Record one inbound DM video fragment from `peer_pubkey`. Returns
    /// `Some(frame)` when the last missing fragment for this `(peer,
    /// stream, frame)` lands; the frame is removed from state on return so
    /// the caller hands it off to the decoder exactly once.
    pub fn record_fragment(
        &self,
        peer_pubkey: &str,
        fragment: DmVideoFragment,
    ) -> Option<AssembledFrame> {
        let DmVideoFragment {
            stream_id,
            frame_seq,
            fragment_index,
            fragment_count,
            keyframe,
            codec,
            timestamp,
            chunk,
        } = fragment;
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
            codec: codec.clone(),
            timestamp,
            chunks: vec![None; fragment_count as usize],
            received_at: Instant::now(),
        });
        // Defensive: a sender shouldn't change fragment_count — or the
        // codec tag — mid-frame (the community reassembler drops the
        // same condition as `CodecMismatch`). If they do, treat it as
        // a fresh frame so chunks of two codecs never merge.
        if entry.fragment_count != fragment_count || entry.codec != codec {
            *entry = PartialFrame {
                fragment_count,
                keyframe,
                codec,
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
                codec: assembled.codec,
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
    /// Codec wire string from the fragments.
    pub codec: String,
    pub timestamp: u32,
    pub data: Vec<u8>,
}

#[cfg(test)]
#[path = "video_tests.rs"]
mod tests;
