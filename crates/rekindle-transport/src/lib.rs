#![forbid(unsafe_code)]
#![recursion_limit = "512"]
// Byte-index string slicing panics inside a multi-byte character; cut with
// `rekindle_utils::text::{prefix, abbreviate}` instead (plan C2).
#![deny(clippy::string_slice)]
//! Unified network transport layer for Rekindle.
//!
//! The transport runs on the Veilid-free ports of `rekindle_records::port`
//! (plan C9.1): records through `RecordPort`, peers' routes and our own
//! through `RoutePort`, app messages and calls through `AppMessagePort`,
//! and inbound network events as `PortEvent`s. It names no Veilid type;
//! `rekindle_protocol::ports` implements the ports over Veilid, and a
//! simulator can implement them in memory.
//!
//! # Module Boundaries
//!
//! - `broadcast/` — all outbound I/O: sends, DHT writes, route management,
//!   node lifecycle.
//! - `subscriptions/` — all inbound I/O: event dispatch, DHT watches,
//!   value change routing.
//! - Everything else (operations, payload, crypto, session, config, etc.) is
//!   business logic and type definitions over those two.

// ── I/O modules (over the ports) ───────────────────────────────────────
pub mod broadcast;
pub mod subscriptions;

// ── Business logic ─────────────────────────────────────────────────────
pub mod config;
pub mod crypto;
pub mod error;
pub mod frame;
pub mod gossip;
pub mod handler;
pub mod operations;
pub mod payload;
pub mod query;
pub mod session;
pub mod shared;

#[cfg(test)]
mod tests;

// ── Public API re-exports ────────────────────────────────────────────

// Core lifecycle (from broadcast/)
pub use broadcast::node::TransportNode;
pub use broadcast::peer_registry::{CircuitSummary, PeerRegistry, PeerSnapshot};
pub use broadcast::send::{AppCaller, BroadcastReport, Sender};
pub use rekindle_records::port::PeerTarget;

// Inbound handler trait
pub use handler::{InboundHandler, TransportEvent, VerifiedSender};

// Gossip (data structures for app-layer mesh management)
pub use gossip::{DedupCache, GossipMesh, LamportClock, OnlineMember};

// Configuration
pub use config::{SafetyConfig, SafetyProfile, TransportConfig};

// Error
pub use error::TransportError;

// Shared state and introspection
pub use crypto::mek::MekCacheEntrySnapshot;
pub use shared::{AttachmentState, SharedState, TransportNotification, TransportSnapshot};

// Crypto — pseudonym, prekeys (the 1:1 ratchet is `rekindle_db::ratchet`,
// plan E2.3)
pub use crypto::prekeys::PreKeyBundle;
pub use crypto::pseudonym::derive_community_pseudonym;

// Session state
pub use session::{CommunityMembership, Session, SessionIdentity};

// Query engine
pub use query::{
    community_detail, list_communities, ChannelOverviewDisplay, CommunityDetail,
    CommunityMetaSummary, CommunityOverview, DecryptedMessageDisplay, QueryEngine, RoleDisplay,
};

// Frame
pub use frame::TypeId;

// Crypto (for app-layer use)
pub use crypto::envelope::{recipient_bytes, sign_payload, verify_signed_payload, Addressing};
pub use crypto::mek::{unwrap_mek, wrap_mek, Mek, MekCache};

// Subscriptions (consolidated inbound signal handling)
pub use subscriptions::events::SubscriptionEvent;
pub use subscriptions::SubscriptionManager;

// Broadcast (consolidated outbound operations)
pub use broadcast::BroadcastManager;

// Payload types
pub use payload::rpc::{
    CallResponse, ChannelEntrySummary, CommunityLeaveNotification, InboundCall, MekTransferPayload,
    SyncRequest, SyncResponse,
};
