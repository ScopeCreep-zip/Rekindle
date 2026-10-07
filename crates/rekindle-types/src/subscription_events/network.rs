//! Network and infrastructure events — attachment, routes, DHT watches.

use serde::{Deserialize, Serialize};

/// What one of our route classes is doing, for a status indicator (plan
/// C7.9c). Mirrors the route owner's state without its payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RouteAvailability {
    /// Not wanted yet (no node, or before the first want).
    Idle,
    /// An allocation is in flight.
    Allocating,
    /// A route is live and published.
    Available,
    /// Veilid said try again; a retry is scheduled.
    Unavailable,
    /// Allocation failed for good; the watchdog's backstop want retries.
    Failed,
}

/// Network infrastructure events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum NetworkEvent {
    /// Network attachment state changed (attached, detached, degraded).
    /// Triggered by: `VeilidUpdate::Attachment` via dispatch loop.
    AttachmentChanged {
        /// Raw Veilid `AttachmentState` string, e.g. `"detached"`,
        /// `"attaching"`, `"attached_good"`. The booleans below collapse
        /// this to two bits; the desktop's network indicator needs the
        /// distinction between "attaching" and "attached_weak", which
        /// neither bit can express.
        attachment_state: String,
        is_attached: bool,
        public_internet_ready: bool,
        /// Whether we hold an allocated private route for receiving.
        /// Attached with no route means nobody can reach us, which is
        /// not visible from the other three fields.
        has_route: bool,
        /// What our media-class route is doing. Voice and video need it,
        /// and the general route is never substituted (plan C7.9c), so a
        /// window must be able to say voice is unavailable.
        media_route: RouteAvailability,
    },
    /// Our own allocated private routes died and need reallocation.
    /// Triggered by: `VeilidUpdate::RouteChange` (dead_routes).
    LocalRoutesDied { count: usize },
    /// Remote peer routes died (imported routes expired).
    /// Triggered by: `VeilidUpdate::RouteChange` (dead_remote_routes).
    RemoteRoutesDied { peer_keys: Vec<String> },
    /// A DHT record's value changed (generic, for records without specific handlers).
    /// Triggered by: `VeilidUpdate::ValueChange` for unregistered record keys.
    ValueChanged {
        record_key: String,
        changed_subkeys: Vec<u32>,
    },
}
