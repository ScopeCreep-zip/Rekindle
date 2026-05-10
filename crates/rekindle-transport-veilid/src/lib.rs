#![forbid(unsafe_code)]
#![recursion_limit = "512"]
//! Veilid transport implementation for the Rekindle messaging platform.
//!
//! This crate is the **sole boundary** between Rekindle and the Veilid network.
//! No other crate in the workspace imports `veilid_core`.
//!
//! Implements the `Transport` trait from `rekindle_types::transport`.
//! Constructed by `rekindle-node` in lifecycle.rs, cast to
//! `Arc<dyn Transport>`, passed to `ChatService`. After construction,
//! no code outside this crate references `VeilidTransport` or `veilid_core`.
//!
//! # Module Boundaries
//!
//! - `broadcast/` — ALL outbound Veilid I/O: sends, DHT writes, route
//!   management, node lifecycle, mesh broadcast.
//! - `subscriptions/` — ALL inbound Veilid I/O: event dispatch (raw bytes
//!   to TransportCallback), DHT watches, poll sweeps, gossip dedup.
//! - `transport_impl.rs` — `Transport` trait implementation wrapping
//!   `TransportNode`.
//! - `config.rs`, `frame.rs`, `gossip.rs`, `shared.rs` — Veilid-specific
//!   configuration, wire framing, gossip mesh, shared state.
//! - `payload/` — Re-exports payload types from `rekindle-types` plus
//!   transport-specific serialization/deserialization functions.
//! - `error.rs` — Veilid-specific error types.

// ── Veilid boundary modules ───────────────────────────────────────
pub mod broadcast;
pub mod subscriptions;

// ── Veilid-specific infrastructure ────────────────────────────────
pub mod config;
pub mod error;
pub mod frame;
pub mod gossip;
pub mod shared;
pub mod transport_impl;

// ── Payload re-exports + transport-specific ser/de ────────────────
pub mod payload;

#[cfg(test)]
mod tests;

// ── Public API ────────────────────────────────────────────────────

// Transport trait implementation
pub use transport_impl::VeilidTransport;

// Core lifecycle
pub use broadcast::node::TransportNode;
pub use broadcast::send::{Sender, Caller, BroadcastReport};
pub use broadcast::peer_route::RouteManager;
pub use broadcast::peer_registry::{PeerRegistry, PeerTarget, CircuitSummary, PeerSnapshot};
pub use broadcast::dht::DhtStore;

// Gossip mesh
pub use gossip::{GossipMesh, OnlineMember, DedupCache, LamportClock};

// Configuration
pub use config::{TransportConfig, SafetyConfig, SafetyProfile};

// Error
pub use error::TransportError as VeilidTransportError;

// Shared state
pub use shared::{SharedState, AttachmentState, TransportNotification, TransportSnapshot};

// Frame
pub use frame::TypeId;

// Time utilities
pub use rekindle_utils::{timestamp_ms, timestamp_secs};

// Broadcast manager
pub use broadcast::BroadcastManager;

// Re-export node::deserialize_keypair for daemon use
pub use broadcast::node::deserialize_keypair;
