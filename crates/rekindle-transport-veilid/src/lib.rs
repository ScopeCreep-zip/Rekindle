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
//! `broadcast/` and `subscriptions/` are crate-internal. They contain all
//! direct veilid_core usage. No veilid_core types appear in the public API.
//!
//! External consumers use the `Transport` trait for DHT, messaging, routes,
//! watches, inspect, and bulk transfer. All trait methods operate on
//! rekindle-types. The concrete `VeilidTransport` converts internally.

// Crate-internal Veilid boundary modules
pub(crate) mod broadcast;
pub(crate) mod subscriptions;

// Public infrastructure
pub mod config;
pub mod error;
pub mod frame;
pub mod gossip;
pub mod shared;
pub mod transport_impl;

// Commoditized delivery
pub mod resolver;
pub mod delivery;
pub mod mesh_manager;
pub mod bulk_transfer;

// Payload re-exports and transport-specific serialization
pub mod payload;

#[cfg(test)]
mod tests;

/// Veilid SMPL schema member key length in bytes.
///
/// Mirrors `veilid_core::storage_manager::MEMBER_ID_LENGTH` which is
/// `pub(crate)` and cannot be imported. Every `DHTSchemaSMPLMember.m_key`
/// must be exactly this many bytes. Veilid schema validation rejects
/// any other length.
pub const VEILID_MEMBER_ID_LENGTH: usize = 32;

// Public API re-exports

// Transport trait implementation
pub use transport_impl::VeilidTransport;

// Node lifecycle
pub use broadcast::node::TransportNode;

// Peer messaging
pub use broadcast::send::{Sender, Caller, BroadcastReport};

// Route lifecycle
pub use broadcast::peer_route::RouteManager;

// Peer registry
pub use broadcast::peer_registry::{PeerRegistry, PeerTarget, CircuitSummary, PeerSnapshot};

// Gossip mesh
pub use gossip::{GossipMesh, OnlineMember, DedupCache, LamportClock};

// Configuration
pub use config::{TransportConfig, SafetyConfig, SafetyProfile, StabilityPreference, SequencingPreference};

// Error taxonomy
pub use error::TransportError as VeilidTransportError;

// Observable shared state
pub use shared::{SharedState, AttachmentState, TransportNotification, TransportSnapshot};

// Wire frame type IDs
pub use frame::TypeId;

// Time utilities
pub use rekindle_utils::{timestamp_ms, timestamp_secs};

// Broadcast manager
pub use broadcast::BroadcastManager;

// Commoditized delivery
pub use resolver::RouteResolver;
pub use delivery::DeliveryEngine;
pub use rekindle_types::transport::{Durability, DeliveryReport};
pub use mesh_manager::MeshManager;

// Bulk file transfer
pub use bulk_transfer::{
    BulkSender, TransferRegistry, TransferFrame, TransferProgress,
    TransferDirection, TransferStatus, TransferError,
    TYPEID_BULK_TRANSFER,
};

// Transport trait and inbound events from rekindle-types
pub use rekindle_types::transport::{
    Transport, InboundEvent, InspectResult, OpenRecord, WatchToken, RecordSchema,
};
