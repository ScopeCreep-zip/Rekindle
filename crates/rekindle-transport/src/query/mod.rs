//! High-level query operations for CLI and TUI consumption.
//!
//! [`QueryEngine`] composes low-level DHT reads, MEK decryption, and
//! profile resolution into display-ready types. Every returned type
//! implements `Serialize + Clone + Debug` and contains no Veilid-internal
//! types — only strings, numbers, and bools.
//!
//! The CLI calls these methods directly for one-shot commands.
//! The TUI calls them from `tokio::spawn` tasks, routing results back
//! through the action channel as `CommandResult` variants.

use std::sync::Arc;

use parking_lot::RwLock;

use crate::broadcast::dht::DhtStore;
use crate::broadcast::peer_registry::PeerRegistry;
use crate::crypto::mek::{MekCache, MekCacheEntrySnapshot};
use crate::shared::SharedState;

// ── Display types (re-exported from rekindle-types) ─────────────────────
//
// These are the SSOT definitions in `rekindle_types::display`. The transport
// crate re-exports them so existing code that imports from here keeps working.
// New code should import from `rekindle_types::display` directly.

pub use rekindle_types::display::{
    ChannelOverviewDisplay, CommunityDetail, CommunityOverview, DecryptedMessageDisplay,
    DmMessageDisplay, DmThreadDisplay, FriendDisplay, RoleDisplay, TransportSnapshot,
};

// ── QueryEngine ─────────────────────────────────────────────────────────

/// High-level query interface for CLI and TUI.
///
/// Obtained via [`TransportNode::query()`](crate::broadcast::node::TransportNode).
/// Composes low-level DHT reads + MEK decryption + profile resolution
/// into display-ready types.
pub struct QueryEngine {
    dht: DhtStore,
    mek_cache: Arc<RwLock<MekCache>>,
    peer_registry: Arc<RwLock<PeerRegistry>>,
}

impl QueryEngine {
    /// Create a new query engine.
    pub fn new(
        dht: DhtStore,
        mek_cache: Arc<RwLock<MekCache>>,
        peer_registry: Arc<RwLock<PeerRegistry>>,
    ) -> Self {
        Self {
            dht,
            mek_cache,
            peer_registry,
        }
    }

    // ── Peer queries ────────────────────────────────────────────────

    /// Snapshot of all known peers for display.
    pub fn peer_snapshot(&self) -> Vec<crate::broadcast::peer_registry::PeerSnapshot> {
        self.peer_registry.read().snapshot()
    }

    // ── MEK queries ─────────────────────────────────────────────────

    /// MEK cache snapshot for a community.
    pub fn mek_cache_snapshot(&self, community_id: &str) -> Vec<MekCacheEntrySnapshot> {
        self.mek_cache.read().snapshot(community_id)
    }

    // ── Doctor queries ──────────────────────────────────────────────

    /// Node health summary for the doctor diagnostic view.
    ///
    /// `route_allocated` requires the route manager which `QueryEngine` doesn't
    /// own. The caller passes it explicitly from `TransportNode::status_snapshot()`.
    pub fn node_health(&self, shared: &SharedState, route_allocated: bool) -> TransportSnapshot {
        TransportSnapshot {
            attachment: shared.attachment_state().to_string(),
            is_attached: shared.is_attached(),
            public_internet_ready: shared.public_internet_ready(),
            uptime_secs: shared.uptime().as_secs(),
            peer_count: self.peer_registry.read().route_count(),
            route_allocated,
            route_age_secs: None,
        }
    }
}

mod communities;
mod display_map;
mod messages;
mod social;
