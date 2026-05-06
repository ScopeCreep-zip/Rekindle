//! Network and infrastructure events — attachment, routes, DHT watches.

use serde::{Deserialize, Serialize};

/// Network infrastructure events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum NetworkEvent {
    /// Network attachment state changed (attached, detached, degraded).
    /// Triggered by: `VeilidUpdate::Attachment` via dispatch loop.
    AttachmentChanged {
        is_attached: bool,
        public_internet_ready: bool,
    },
    /// Our own allocated private routes died and need reallocation.
    /// Triggered by: `VeilidUpdate::RouteChange` (dead_routes).
    LocalRoutesDied {
        count: usize,
    },
    /// Remote peer routes died (imported routes expired).
    /// Triggered by: `VeilidUpdate::RouteChange` (dead_remote_routes).
    RemoteRoutesDied {
        peer_keys: Vec<String>,
    },
    /// A DHT watch was successfully renewed.
    /// Triggered by: watch renewal loop in `watches.rs`.
    WatchRenewed {
        record_key: String,
    },
    /// A DHT watch died and was re-established.
    /// Triggered by: `VeilidUpdate::ValueChange` with count=0 or empty subkeys.
    WatchReestablished {
        record_key: String,
    },
    /// A DHT watch renewal or re-establishment failed.
    WatchFailed {
        record_key: String,
        error: String,
    },
    /// A DHT record's value changed (generic, for records without specific handlers).
    /// Triggered by: `VeilidUpdate::ValueChange` for unregistered record keys.
    ValueChanged {
        record_key: String,
        changed_subkeys: Vec<u32>,
    },
}
