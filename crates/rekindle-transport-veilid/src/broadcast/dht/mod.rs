//! Raw DHT record operations — the Veilid boundary for all persistent storage.
//!
//! [`DhtStore`] wraps a Veilid `RoutingContext` configured with the DHT safety
//! profile. It provides access to raw record primitives (create, open, close,
//! get, set, watch, inspect) and `DhtLog` (append-only log built on DHT records).
//!
//! Typed wrappers (profile, governance, registry, etc.) have been removed —
//! the Transport trait's `read_record`/`write_record` provides opaque byte
//! access, and `rekindle-chat` handles JSON serialization/deserialization of
//! application types.

pub mod record;
pub mod short_array;
pub mod channel_log;

use veilid_core::RoutingContext;

/// Entry point for raw DHT record operations.
///
/// Obtained via [`TransportNode::dht()`](crate::broadcast::node::TransportNode::dht).
/// Used internally by `dht_writes.rs` for all DHT I/O.
pub struct DhtStore {
    rc: RoutingContext,
}

impl DhtStore {
    pub(crate) fn new(rc: RoutingContext) -> Self {
        Self { rc }
    }

    /// Access the underlying routing context for raw DHT operations.
    pub fn routing_context(&self) -> &RoutingContext {
        &self.rc
    }
}
