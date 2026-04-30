#![recursion_limit = "512"]
//! Unified network transport layer for Rekindle.
//!
//! This crate is the **sole boundary** between Rekindle and the Veilid network.
//! No other crate in the workspace imports `veilid_core`. All network I/O —
//! peer messaging, DHT record operations, route management, gossip broadcast,
//! and voice transport — flows through the types and traits defined here.
//!
//! # Design Principles
//!
//! 1. **Veilid is an implementation detail.** No Veilid types leak through the
//!    public API. Consumers depend on `rekindle-transport` types only.
//! 2. **Fail closed.** Every inbound message is authenticated before dispatch.
//!    Unsigned or malformed payloads are dropped and logged.
//! 3. **Defense in depth.** Application-layer encryption on every payload class,
//!    even when Veilid provides transport encryption.
//! 4. **Deterministic dispatch.** Binary framing with explicit type tags. No
//!    trial parsing, no byte sniffing, no format guessing.
//! 5. **Configurable privacy.** Safety routing parameters are user-facing
//!    settings, not hardcoded constants.

pub mod config;
pub mod error;
pub mod frame;

pub mod node;
pub mod dispatch;
pub mod handler;

pub mod send;
pub mod route;
pub mod peer;
pub mod gossip;

pub mod dht;
pub mod crypto;
pub mod payload;
pub mod community;

#[cfg(test)]
mod tests;

// ── Public API re-exports ────────────────────────────────────────────

// Core lifecycle
pub use node::TransportNode;
pub use handler::{InboundHandler, VerifiedSender, TransportEvent};

// Outbound
pub use send::{Sender, Caller, BroadcastReport};

// Routing and peers
pub use route::RouteManager;
pub use peer::{PeerRegistry, PeerTarget};

// Gossip (data structures for app-layer mesh management)
pub use gossip::{GossipMesh, OnlineMember, DedupCache, LamportClock};

// DHT
pub use dht::DhtStore;

// Configuration
pub use config::{TransportConfig, SafetyConfig, SafetyProfile};

// Error
pub use error::TransportError;

// Frame
pub use frame::TypeId;

// Crypto (for app-layer use: signing gossip, MEK operations, voice crypto)
pub use crypto::envelope::{sign_payload, verify_signed_payload, sign_gossip_envelope, verify_gossip_envelope};
pub use crypto::voice_crypto::VoiceSessionKey;
pub use crypto::mek::{Mek, MekCache, wrap_mek, unwrap_mek};

// Payload types
pub use payload::dm::DmPayload;
pub use payload::gossip::{GossipPayload, ControlPayload, SignedGossipEnvelope};
pub use payload::voice::VoicePayload;
pub use payload::rpc::{
    BootstrapRequest, BootstrapResponse,
    MekTransferRequest, SyncRequest, SyncResponse,
    InboundCall, CallResponse,
};
