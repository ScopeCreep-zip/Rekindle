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
use std::time::{Duration, Instant};

use parking_lot::Mutex;

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
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Record one inbound DM video fragment. Returns `Some(frame)` when
    /// the last missing fragment for this `(peer, stream, frame)` lands;
    /// the frame is removed from state on return so the caller hands it
    /// off to the decoder exactly once.
    #[allow(
        clippy::too_many_arguments,
        reason = "DmVideoFragment wire envelope unpacks to 8 fields; bundling would only move the args from dispatcher to constructor"
    )]
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

/// Dispatch a completed `AssembledFrame` to the frontend via
/// `DmEvent::VideoFrameAssembled`. Centralizes the event-emit shape
/// so the message_service dispatcher doesn't need to know the wire
/// format (base64 + JSON layout is the adapter's concern).
pub fn dispatch_assembled_frame<D: crate::deps::DmDeps + ?Sized>(
    deps: &D,
    sender_public_key_hex: &str,
    frame: AssembledFrame,
) {
    deps.emit_event(crate::deps::DmEvent::VideoFrameAssembled {
        sender_public_key_hex: sender_public_key_hex.to_string(),
        stream_id: frame.stream_id,
        frame_seq: frame.frame_seq,
        keyframe: frame.keyframe,
        timestamp: frame.timestamp,
        data: frame.data,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    const PEER: &str = "peer_pubkey_hex";
    const SID: [u8; 16] = [0xaa; 16];

    #[test]
    fn single_fragment_completes_frame() {
        let state = DmVideoReassemblyState::new();
        let frame = state.record_fragment(PEER, SID, 1, 0, 1, true, 100, vec![1, 2, 3]);
        let assembled = frame.expect("single-fragment frame completes on first call");
        assert_eq!(assembled.frame_seq, 1);
        assert!(assembled.keyframe);
        assert_eq!(assembled.timestamp, 100);
        assert_eq!(assembled.data, vec![1, 2, 3]);
    }

    #[test]
    fn multi_fragment_assembles_in_order() {
        let state = DmVideoReassemblyState::new();
        assert!(state
            .record_fragment(PEER, SID, 2, 0, 3, false, 200, vec![1, 2])
            .is_none());
        assert!(state
            .record_fragment(PEER, SID, 2, 1, 3, false, 200, vec![3, 4])
            .is_none());
        let assembled = state
            .record_fragment(PEER, SID, 2, 2, 3, false, 200, vec![5, 6])
            .expect("final fragment completes frame");
        assert_eq!(assembled.data, vec![1, 2, 3, 4, 5, 6]);
    }

    #[test]
    fn multi_fragment_out_of_order_still_assembles() {
        let state = DmVideoReassemblyState::new();
        assert!(state
            .record_fragment(PEER, SID, 3, 2, 3, false, 300, vec![5, 6])
            .is_none());
        assert!(state
            .record_fragment(PEER, SID, 3, 0, 3, false, 300, vec![1, 2])
            .is_none());
        let assembled = state
            .record_fragment(PEER, SID, 3, 1, 3, false, 300, vec![3, 4])
            .expect("frame completes regardless of arrival order");
        assert_eq!(assembled.data, vec![1, 2, 3, 4, 5, 6]);
    }

    #[test]
    fn invalid_fragment_index_returns_none() {
        let state = DmVideoReassemblyState::new();
        // fragment_index == fragment_count is out-of-range.
        assert!(state
            .record_fragment(PEER, SID, 1, 2, 2, false, 0, vec![1])
            .is_none());
        // fragment_count = 0 is malformed.
        assert!(state
            .record_fragment(PEER, SID, 1, 0, 0, false, 0, vec![1])
            .is_none());
    }

    #[test]
    fn forget_peer_drops_all_state() {
        let state = DmVideoReassemblyState::new();
        // Partial frame in flight for PEER.
        assert!(state
            .record_fragment(PEER, SID, 4, 0, 2, false, 400, vec![1, 2])
            .is_none());
        state.forget_peer(PEER);
        // Next fragment looks like a fresh frame after forget — it
        // should NOT auto-complete because state was cleared.
        assert!(state
            .record_fragment(PEER, SID, 4, 1, 2, false, 400, vec![3, 4])
            .is_none());
    }

    #[test]
    fn changing_fragment_count_mid_frame_resets() {
        let state = DmVideoReassemblyState::new();
        assert!(state
            .record_fragment(PEER, SID, 5, 0, 3, false, 500, vec![1, 2])
            .is_none());
        // Sender suddenly claims a different fragment_count for the
        // same frame — treat as a fresh frame.
        let result = state.record_fragment(PEER, SID, 5, 0, 1, false, 500, vec![9, 9]);
        assert!(result.is_some()); // new fragment_count=1, fragment_index=0 → completes
        assert_eq!(result.unwrap().data, vec![9, 9]);
    }

    #[test]
    fn different_streams_track_independently() {
        let state = DmVideoReassemblyState::new();
        let sid_a = [0xaa; 16];
        let sid_b = [0xbb; 16];
        assert!(state
            .record_fragment(PEER, sid_a, 1, 0, 2, false, 600, vec![1, 2])
            .is_none());
        // Different stream, same frame_seq — should NOT auto-complete A.
        let result = state.record_fragment(PEER, sid_b, 1, 0, 1, false, 700, vec![9]);
        assert!(result.is_some());
        assert_eq!(result.unwrap().stream_id, sid_b);
        // A still pending, complete with the second fragment.
        let result_a = state.record_fragment(PEER, sid_a, 1, 1, 2, false, 600, vec![3, 4]);
        assert!(result_a.is_some());
        assert_eq!(result_a.unwrap().stream_id, sid_a);
    }

    #[test]
    fn fragment_payload_limit_is_28_kib() {
        // Sanity check the published constant — frontend keyframe
        // splitter depends on this value.
        assert_eq!(FRAGMENT_PAYLOAD_LIMIT, 28 * 1024);
    }
}
