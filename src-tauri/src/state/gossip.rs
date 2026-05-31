//! Phase 23.B — extracted from `state.rs`. Per-community gossip overlay
//! state stored inside `CommunityState.gossip: Option<GossipOverlay>`.

use std::collections::{HashMap, VecDeque};

/// An online community member with their route blob and last-seen timestamp.
#[derive(Debug, Clone)]
pub struct OnlineMember {
    /// Veilid private route blob for reaching this member.
    pub route_blob: Vec<u8>,
    /// Last advertised member status from the registry or gossip mesh.
    pub status: String,
    /// Timestamp (seconds since epoch) of last valid gossip message or presence update.
    /// Used for TTL-based eviction of stale members.
    pub last_seen: u64,
}

/// Gossip overlay state for a community.
///
/// Each member maintains a random peer set of D online members and forwards
/// received messages to them. Adaptive degree:
/// - ≤20 members: D = N-1 (direct mesh)
/// - 21-60: D = 6, 61+: D = 8
#[derive(Debug, Clone)]
pub struct GossipOverlay {
    /// Current gossip peers: pseudonym_key → member info.
    /// These are the D peers we send/forward every message to.
    pub peers: HashMap<String, OnlineMember>,
    /// All online members: pseudonym_key → member info.
    /// Superset of `peers`. Updated on each presence poll.
    pub online_members: HashMap<String, OnlineMember>,
    /// Lamport counter for outgoing messages.
    /// Incremented for each message we originate (not forwards).
    pub lamport_counter: u64,
    /// True until the first successful sync after coming online.
    /// Used to trigger a `SyncRequest` to online peers for catch-up.
    pub needs_initial_sync: bool,
    /// A1/P4.1 — broadcasts queued because `peers` was empty at send time.
    /// `send_to_mesh_raw` enqueues here instead of dropping; the next
    /// presence poll that lands online peers drains the queue and re-sends.
    /// Bounded at 100 (oldest dropped) so an offline burst doesn't OOM.
    /// Without this, the first member of a fresh community broadcast all
    /// their join announcements / MEK requests / governance updates to a
    /// zero-peer mesh — silently lost.
    pub pending_mesh_broadcasts:
        VecDeque<rekindle_protocol::dht::community::envelope::SignedEnvelope>,
}

impl Default for GossipOverlay {
    fn default() -> Self {
        Self {
            peers: HashMap::new(),
            online_members: HashMap::new(),
            lamport_counter: 0,
            needs_initial_sync: true,
            pending_mesh_broadcasts: VecDeque::with_capacity(16),
        }
    }
}
