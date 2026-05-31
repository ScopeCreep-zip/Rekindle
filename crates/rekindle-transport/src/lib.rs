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
pub mod community;
pub mod config;
pub mod crypto;
pub mod envelope_queue;
pub mod envelope_store;
pub mod error;
pub mod frame;
pub mod friend_store;
pub mod gossip;
pub mod handler;
pub mod operations;
pub mod payload;
pub mod query;
pub mod seq_tracker;
pub mod session;
pub mod shared;

#[cfg(test)]
mod tests;

// ── Public API re-exports ────────────────────────────────────────────

// Core lifecycle (from broadcast/)
pub use broadcast::dht::DhtStore;
pub use broadcast::node::TransportNode;
pub use broadcast::peer_registry::{CircuitSummary, PeerRegistry, PeerSnapshot, PeerTarget};
pub use broadcast::peer_route::RouteManager;
pub use broadcast::send::{BroadcastReport, Caller, Sender};

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

// Crypto — pseudonym, Signal, prekeys
pub use crypto::prekeys::PreKeyBundle;
pub use crypto::pseudonym::derive_community_pseudonym;
pub use crypto::signal_session::{SessionInitInfo, SignalSessionManager};
pub use crypto::signal_store::{
    IdentityKeyStore, MemoryIdentityStore, MemoryPreKeyStore, MemorySessionStore, PreKeyStore,
    SessionStore,
};

// Session state
pub use session::{CommunityMembership, PendingFriendRequest, Session, SessionIdentity};

// W16.1 — envelope reliability primitive contract + default impls
pub use envelope_store::{
    EnvelopeKind, EnvelopeStore, JsonEnvelopeStore, MemoryEnvelopeStore, PendingEnvelope,
    PersistedCallState, StoreError,
};

// W16.2 — envelope retry queue (fire-and-forget + expect-reply)
pub use envelope_queue::{EnvelopeQueue, QueueError, RetryConfig, DEFAULT_REPLY_TIMEOUT};

// W16.3 — receiver-side dedup
pub use seq_tracker::SeqTracker;

// Track A.1 — Receive-path friend authority (Phase 2 DHT-Inbox Pivot)
pub use friend_store::{FriendRecord, FriendStatus, FriendStore, MemoryFriendStore};

// Query engine
pub use query::{
    ChannelOverviewDisplay, CommunityDetail, CommunityOverview, DecryptedMessageDisplay,
    DmMessageDisplay, DmThreadDisplay, FriendDisplay, QueryEngine, RoleDisplay,
};

// Frame
pub use frame::TypeId;

// Time utilities
pub use rekindle_utils::{timestamp_ms, timestamp_secs};

// Crypto (for app-layer use)
pub use crypto::envelope::{
    sign_gossip_envelope, sign_payload, verify_gossip_envelope, verify_signed_payload,
};
pub use crypto::mek::{unwrap_mek, wrap_mek, Mek, MekCache};
pub use crypto::voice_crypto::VoiceSessionKey;

// Subscriptions (consolidated inbound signal handling)
pub use subscriptions::events::SubscriptionEvent;
pub use subscriptions::SubscriptionManager;

// Broadcast (consolidated outbound operations)
pub use broadcast::BroadcastManager;

// Payload types
pub use payload::dm::DmPayload;
pub use payload::gossip::{ControlPayload, GossipPayload, SignedGossipEnvelope};
pub use payload::rpc::{
    CallResponse, ChannelEntrySummary, CommunityLeaveNotification, GovernanceOp,
    GovernanceOpResponse, GovernanceRequest, InboundCall, MekTransferPayload, SyncRequest,
    SyncResponse,
};
pub use payload::voice::VoicePayload;

// Re-export node::deserialize_keypair for daemon use
pub use broadcast::node::deserialize_keypair;
