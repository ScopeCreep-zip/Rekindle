//! Transport trait definitions and supporting types.
//!
//! This module defines the `Transport` and `TransportCallback` traits that
//! all transport backends implement. It lives in `rekindle-types` (the bottom
//! of the dependency graph) so that:
//!
//! - Transport backend crates (`rekindle-transport-veilid`, etc.) can import
//!   and implement the trait without circular dependencies.
//! - `rekindle-transport` can depend on backend crates AND re-export the trait,
//!   acting as a feature-gated registry of all available backends.
//! - `rekindle-chat` programs against the trait without knowing which backend
//!   is active.
//!
//! Every method operates on opaque bytes. Transport never inspects, parses,
//! encrypts, or decrypts payload content.

use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;

// ── Error ──────────────────────────────────────────────────────────

pub type TransportResult<T> = Result<T, TransportError>;

#[derive(Error, Debug)]
pub enum TransportError {
    #[error("transport not attached")]
    NotAttached,

    #[error("transport start failed: {0}")]
    StartFailed(String),

    #[error("send failed: {reason}")]
    SendFailed { reason: String },

    #[error("record operation failed: {reason}")]
    RecordFailed { reason: String },

    #[error("route allocation failed: {reason}")]
    RouteFailed { reason: String },

    #[error("broadcast failed: {reason}")]
    BroadcastFailed { reason: String },

    #[error("peer not reachable: {peer_key}")]
    PeerUnreachable { peer_key: String },

    #[error("record not found: {key}")]
    RecordNotFound { key: String },

    #[error("timeout after {seconds}s")]
    Timeout { seconds: u64 },

    #[error("serialization failed: {0}")]
    Serialization(String),

    #[error("deserialization failed: {0}")]
    Deserialization(String),

    #[error("internal: {0}")]
    Internal(String),
}

// ── Events ─────────────────────────────────────────────────────────

/// Events emitted by the transport layer to the callback.
/// Transport-internal state changes, not application events.
#[derive(Debug, Clone)]
pub enum TransportEvent {
    /// Transport has attached to the network and is ready.
    Attached,
    /// Transport has detached from the network.
    Detached,
    /// A new route was allocated.
    RouteAllocated { route_id: String },
    /// A previously allocated route has died.
    RouteDied { route_id: String },
    /// A watch on a record has expired and needs renewal.
    WatchExpired { record_key: String },
    /// The number of known peers has changed.
    PeerCountChanged { count: u32 },
    /// Public internet reachability changed.
    PublicInternet { available: bool },
}

// ── Types ──────────────────────────────────────────────────────────

/// Record schema for `Transport::create_record`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RecordSchema {
    /// Single-writer record with N subkeys (Veilid: DFLT).
    SingleWriter { subkey_count: u32 },
    /// Multi-writer record (Veilid: SMPL).
    MultiWriter {
        owner_subkeys: u16,
        member_subkeys: u16,
        member_count: u16,
    },
}

/// Report from a community broadcast operation.
#[derive(Debug, Clone, Default)]
pub struct BroadcastReport {
    pub peers_sent: u32,
    pub peers_failed: u32,
}

/// Opaque token returned by `Transport::watch_record`.
/// Used to cancel the watch via `Transport::cancel_watch`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WatchToken(pub u64);

// ── Traits ─────────────────────────────────────────────────────────

/// Transport-agnostic interface for sending and receiving bytes.
///
/// Implementations: `rekindle-transport-veilid` (Veilid DHT + gossip),
/// future: `rekindle-transport-matrix`, `rekindle-transport-signal`.
///
/// `rekindle-chat` programs exclusively against this trait. Swapping
/// transport means providing a different implementation — zero chat
/// code changes.
#[async_trait]
pub trait Transport: Send + Sync + 'static {
    // ── Lifecycle ───────────────────────────────────────────

    /// Start the transport (attach to network). Some backends start at
    /// construction time and this is a no-op.
    async fn start(&self) -> TransportResult<()>;

    /// Graceful shutdown: stop background tasks, release routes, detach.
    /// Non-consuming — the transport is held in `Arc` by multiple owners.
    async fn shutdown(&self) -> TransportResult<()>;

    /// Install the callback that receives inbound events.
    ///
    /// Called once after `ChatService` construction. The callback is
    /// `ChatService`'s `EventRouter` which implements `TransportCallback`.
    ///
    /// Transport backends that buffer events before the callback is
    /// installed (e.g., Veilid's arc_swap pattern) drain the buffer
    /// on the next dispatch loop iteration after this call.
    fn set_callback(&self, callback: Arc<dyn TransportCallback>);

    /// Whether the transport is attached to the network and ready.
    fn is_attached(&self) -> bool;

    // ── Peer messaging (opaque bytes) ──────────────────────
    async fn send_to_peer(&self, peer_key: &str, data: &[u8]) -> TransportResult<()>;
    async fn call_peer(&self, peer_key: &str, data: &[u8]) -> TransportResult<Vec<u8>>;

    // ── Persistent records ─────────────────────────────────
    async fn create_record(&self, schema: RecordSchema) -> TransportResult<(String, Vec<u8>)>;
    async fn open_record(&self, key: &str, writer: Option<&[u8]>) -> TransportResult<()>;
    async fn write_record(
        &self, key: &str, subkey: u32, data: &[u8], writer: Option<&[u8]>,
    ) -> TransportResult<()>;
    async fn read_record(
        &self, key: &str, subkey: u32, force_refresh: bool,
    ) -> TransportResult<Option<Vec<u8>>>;
    async fn watch_record(&self, key: &str, subkeys: &[u32]) -> TransportResult<WatchToken>;
    async fn cancel_watch(&self, token: WatchToken) -> TransportResult<()>;
    async fn inspect_record(
        &self, key: &str, subkeys: &[u32],
    ) -> TransportResult<Vec<Option<u32>>>;
    async fn close_record(&self, key: &str) -> TransportResult<()>;

    // ── Route management ───────────────────────────────────
    async fn allocate_route(&self) -> TransportResult<(String, Vec<u8>)>;
    fn route_blob(&self) -> Option<Vec<u8>>;
    fn cache_peer_route(&self, peer_key: &str, route_blob: Vec<u8>);
    fn invalidate_peer_route(&self, peer_key: &str);
    async fn import_route(&self, route_blob: &[u8]) -> TransportResult<String>;

    // ── Community broadcast ────────────────────────────────
    async fn broadcast(
        &self, community_id: &str, data: &[u8],
    ) -> TransportResult<BroadcastReport>;
    async fn join_mesh(&self, community_id: &str) -> TransportResult<()>;
    async fn leave_mesh(&self, community_id: &str) -> TransportResult<()>;

    // ── Diagnostics ────────────────────────────────────────
    fn peer_count(&self) -> u32;
    fn attachment_state(&self) -> &str;
    fn uptime_secs(&self) -> u64;
}

/// Callback trait for inbound events from the transport layer.
///
/// Implemented by `rekindle-chat`'s `EventRouter`. Transport calls
/// these methods with raw bytes — no parsing, no deserialization.
#[async_trait]
pub trait TransportCallback: Send + Sync + 'static {
    /// Opaque bytes arrived from a peer via app_message.
    async fn on_message(&self, sender_key: &str, data: &[u8]);

    /// Opaque bytes arrived via app_call, expecting a reply.
    async fn on_call(&self, sender_key: &str, data: &[u8]) -> Vec<u8>;

    /// A watched record changed.
    async fn on_record_change(
        &self, record_key: &str, subkeys: Vec<u32>,
        value_count: u32, data: Option<Vec<u8>>,
    );

    /// Transport-level event (attach, detach, route death, etc).
    async fn on_event(&self, event: TransportEvent);
}
