//! Phase 16 — Per-community video reassembly state.
//!
//! Wraps `Reassembler` (per-community) + the local sender's announced
//! stream-id set + per-stream Lamport clock for `TopologyChange`
//! LWW. Hosted on AppState (src-tauri); methods here are pure and
//! parking_lot-locked so multiple concurrent senders + receivers
//! don't collide.

use std::collections::{HashMap, HashSet};

use parking_lot::Mutex;

use crate::reassembler::{ReassembledFrame, Reassembler};
use crate::{VideoFragment, VideoParityFragment};

/// Per-community reassembly state plus a set of stream_ids the local
/// sender has already announced via `TopologyChange`. Lives on
/// `AppState` so multiple active video streams across communities
/// don't collide.
#[derive(Default)]
pub struct VideoReassemblyState {
    inner: Mutex<HashMap<String, Reassembler>>,
    /// `(community_id, stream_id)` pairs we've broadcast a
    /// `TopologyChange { reason: "initial" }` for. Cleared on logout
    /// via `clear()`. Bounded growth: one entry per concurrent
    /// outbound video stream — typically 1 or 2 per community.
    started_streams: Mutex<HashSet<(String, [u8; 16])>>,
    /// Architecture §10.6 + §22 — per-stream Lamport clock of the
    /// last accepted `TopologyChange`. When two peers simultaneously
    /// switch the relay (mesh ↔ SFU), only the higher-lamport entry
    /// wins; lower-lamport messages are dropped so the reassembler
    /// doesn't flap between relays. Keyed by `(community_id, stream_id)`.
    last_topology_lamport: Mutex<HashMap<(String, [u8; 16]), u64>>,
}

impl VideoReassemblyState {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Return true if this is the first call for `(community_id,
    /// stream_id)` since startup — used by `send_video_frame` to emit
    /// exactly one `TopologyChange { reason: "initial" }` per stream.
    pub fn mark_stream_started(&self, community_id: &str, stream_id: [u8; 16]) -> bool {
        let mut set = self.started_streams.lock();
        set.insert((community_id.to_string(), stream_id))
    }

    /// Ingest a data fragment for `community_id` from `sender_hex` and
    /// return the reassembled frame when complete. Errors map to
    /// `tracing::warn!` at the caller — they're recoverable; we just
    /// drop malformed fragments.
    pub fn ingest(
        &self,
        community_id: &str,
        sender_hex: &str,
        fragment: VideoFragment,
        now_ms: u32,
    ) -> Option<ReassembledFrame> {
        let mut map = self.inner.lock();
        let reassembler = map.entry(community_id.to_string()).or_default();
        match reassembler.ingest(sender_hex, fragment, now_ms) {
            Ok(frame) => frame,
            Err(e) => {
                tracing::warn!(community = %community_id, error = %e, "video fragment dropped");
                None
            }
        }
    }

    /// Ingest an FEC parity fragment. Returns `Some(frame)` if the
    /// parity completes the frame's reconstruction (combined with
    /// already-received data fragments).
    pub fn ingest_parity(
        &self,
        community_id: &str,
        sender_hex: &str,
        fragment: VideoParityFragment,
        now_ms: u32,
    ) -> Option<ReassembledFrame> {
        let mut map = self.inner.lock();
        let reassembler = map.entry(community_id.to_string()).or_default();
        match reassembler.ingest_parity(sender_hex, fragment, now_ms) {
            Ok(frame) => frame,
            Err(e) => {
                tracing::warn!(community = %community_id, error = %e, "video parity fragment dropped");
                None
            }
        }
    }

    /// Drop pending fragments for a stream — invoked when the local
    /// receiver sends a `KeyframeRequest`.
    pub fn reset_stream(&self, community_id: &str, stream_id: [u8; 16], sender_hex: &str) {
        let mut map = self.inner.lock();
        if let Some(reassembler) = map.get_mut(community_id) {
            reassembler.reset_stream(stream_id, sender_hex);
        }
    }

    /// Forget a community's reassembly state on logout / leave.
    pub fn forget(&self, community_id: &str) {
        self.inner.lock().remove(community_id);
        self.started_streams
            .lock()
            .retain(|(cid, _)| cid != community_id);
        self.last_topology_lamport
            .lock()
            .retain(|(cid, _), _| cid != community_id);
    }

    pub fn clear(&self) {
        self.inner.lock().clear();
        self.started_streams.lock().clear();
        self.last_topology_lamport.lock().clear();
    }

    /// Architecture §10.6 + §22 tie-break — return `true` when the
    /// incoming `TopologyChange` lamport is strictly greater than the
    /// last-seen lamport for `(community, stream)`. Updates the stored
    /// lamport on accept; rejects (returns false) for stale or
    /// duplicate-lamport messages, eliminating the flap that occurs
    /// when two peers race a relay handover.
    pub fn accept_topology_change(
        &self,
        community_id: &str,
        stream_id: [u8; 16],
        lamport: u64,
    ) -> bool {
        let key = (community_id.to_string(), stream_id);
        let mut map = self.last_topology_lamport.lock();
        let entry = map.entry(key).or_insert(0);
        if lamport > *entry {
            *entry = lamport;
            true
        } else {
            false
        }
    }
}
