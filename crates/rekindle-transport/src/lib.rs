#![forbid(unsafe_code)]
#![recursion_limit = "512"]
//! Unified network transport layer for Rekindle.
//!
//! This crate is the **sole boundary** between Rekindle and the Veilid network.
//! No other crate in the workspace imports `veilid_core`.
//!
//! # Module Boundaries
//!
//! - `broadcast/` — ALL outbound Veilid I/O: sends, DHT writes, route management,
//!   node lifecycle. The only module that imports `veilid_core` for outbound ops.
//! - `subscriptions/` — ALL inbound Veilid I/O: event dispatch, DHT watches,
//!   value change routing. The only module that imports `veilid_core` for inbound ops.
//! - Everything else (operations, payload, crypto, session, config, etc.) contains
//!   zero `veilid_core` imports — pure business logic and type definitions.

// ── Veilid boundary modules (sole veilid_core importers) ───────────────
pub mod broadcast;
pub mod subscriptions;

// ── Business logic (zero veilid imports) ───────────────────────────────
pub mod config;
pub mod error;
pub mod frame;
pub mod shared;
pub mod session;
pub mod query;
pub mod handler;
pub mod gossip;
pub mod crypto;
pub mod payload;
pub mod community;
pub mod operations;

#[cfg(test)]
mod tests;

// ── Public API re-exports ────────────────────────────────────────────

// Core lifecycle (from broadcast/)
pub use broadcast::node::TransportNode;
pub use broadcast::send::{Sender, Caller, BroadcastReport};
pub use broadcast::peer_route::RouteManager;
pub use broadcast::peer_registry::{PeerRegistry, PeerTarget, CircuitSummary, PeerSnapshot};
pub use broadcast::dht::DhtStore;

// Inbound handler trait
pub use handler::{InboundHandler, VerifiedSender, TransportEvent};

// Gossip (data structures for app-layer mesh management)
pub use gossip::{GossipMesh, OnlineMember, DedupCache, LamportClock};

// Configuration
pub use config::{TransportConfig, SafetyConfig, SafetyProfile};

// Error
pub use error::TransportError;

// Shared state and introspection
pub use shared::{SharedState, AttachmentState, TransportNotification, TransportSnapshot};
pub use crypto::mek::MekCacheEntrySnapshot;

// Crypto — pseudonym, Signal, prekeys
pub use crypto::pseudonym::derive_community_pseudonym;
pub use crypto::prekeys::PreKeyBundle;
pub use crypto::signal_session::{SignalSessionManager, SessionInitInfo};
pub use crypto::signal_store::{
    IdentityKeyStore, PreKeyStore, SessionStore,
    MemoryIdentityStore, MemoryPreKeyStore, MemorySessionStore,
};

// Session state
pub use session::{Session, SessionIdentity, CommunityMembership, PendingFriendRequest};

// Query engine
pub use query::{
    QueryEngine,
    CommunityOverview, CommunityDetail,
    ChannelOverviewDisplay, DecryptedMessageDisplay,
    FriendDisplay, DmThreadDisplay, DmMessageDisplay,
    RoleDisplay,
};

// Frame
pub use frame::TypeId;

// Time utilities
pub use rekindle_utils::{timestamp_ms, timestamp_secs};

// Crypto (for app-layer use)
pub use crypto::envelope::{sign_payload, verify_signed_payload, sign_gossip_envelope, verify_gossip_envelope};
pub use crypto::voice_crypto::VoiceSessionKey;
pub use crypto::mek::{Mek, MekCache, wrap_mek, unwrap_mek};

// Subscriptions (consolidated inbound signal handling)
pub use subscriptions::SubscriptionManager;
pub use subscriptions::events::SubscriptionEvent;

// Broadcast (consolidated outbound operations)
pub use broadcast::BroadcastManager;

// Payload types
pub use payload::dm::DmPayload;
pub use payload::gossip::{GossipPayload, ControlPayload, SignedGossipEnvelope};
pub use payload::voice::VoicePayload;
pub use payload::rpc::{
    MekTransferPayload, ChannelEntrySummary,
    CommunityLeaveNotification, GovernanceRequest, GovernanceOp, GovernanceOpResponse,
    SyncRequest, SyncResponse,
    InboundCall, CallResponse,
};

// Re-export node::deserialize_keypair for daemon use
pub use broadcast::node::deserialize_keypair;
