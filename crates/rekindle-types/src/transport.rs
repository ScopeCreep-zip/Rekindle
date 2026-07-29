//! Transport trait definitions and supporting types.
//!
//! This module defines the `Transport` trait that all transport backends
//! implement, plus the `InboundEvent` enum that replaces the old callback
//! trait. It lives in `rekindle-types` (the bottom of the dependency graph)
//! so that:
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
//!
//! Inbound data flows through `mpsc::Sender<InboundEvent>` — the transport
//! sends typed events, the chat layer reads them. No callback trait, no lazy
//! installation, no RwLock, no circular dependency.

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

// ── Transport Events ──────────────────────────────────────────────

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

// ── Inbound Event ─────────────────────────────────────────────────

/// Typed inbound event from the transport layer to the chat layer.
///
/// Replaces the old `TransportCallback` trait. The transport sends these
/// to an `mpsc::Sender<InboundEvent>` that the chat layer reads from.
/// No trait object, no lazy installation, no RwLock, no circular dependency.
///
/// The `Call` variant includes a `reply_tx` oneshot so the chat layer can
/// send the response bytes back to the transport, which handles the
/// transport-specific reply mechanism (e.g., Veilid's `app_call_reply`).
/// No transport API types leak to the chat layer.
#[derive(Debug)]
pub enum InboundEvent {
    /// Opaque bytes arrived from a peer (app_message equivalent).
    /// The chat layer parses TypeId, verifies signatures, dispatches.
    Message {
        sender_key: String,
        data: Vec<u8>,
    },

    /// Opaque bytes arrived expecting a reply (app_call equivalent).
    /// Chat layer sends response bytes via `reply_tx`. The transport
    /// handles the transport-specific reply mechanism internally.
    Call {
        sender_key: String,
        data: Vec<u8>,
        reply_tx: tokio::sync::oneshot::Sender<Vec<u8>>,
    },

    /// A watched DHT record changed.
    RecordChange {
        record_key: String,
        subkeys: Vec<u32>,
        count: u32,
        data: Option<Vec<u8>>,
    },

    /// Transport-level lifecycle event (attach, detach, route death, etc).
    Event(TransportEvent),

    /// Bulk file transfer progress update. Emitted by the transport's
    /// internal TransferRegistry as chunks arrive and are verified.
    TransferProgress {
        transfer_id: [u8; 16],
        filename: String,
        total_size: u64,
        bytes_transferred: u64,
        chunks_received: u32,
        chunk_count: u32,
        status: TransferStatus,
    },

    /// A peer is offering to send us a file. The transport auto-accepts
    /// files under the configured size threshold. Large files are reported
    /// here so the chat/TUI layer can prompt the user.
    TransferOffer {
        transfer_id: [u8; 16],
        sender_peer_key: String,
        filename: String,
        total_size: u64,
        media_type: String,
    },

    /// A bulk file transfer completed and was verified.
    TransferComplete {
        transfer_id: [u8; 16],
        path: String,
        hash_match: bool,
    },

    /// A bulk file transfer failed.
    TransferFailed {
        transfer_id: [u8; 16],
        reason: String,
    },
}

// ── Types ──────────────────────────────────────────────────────────

/// Record schema for `Transport::create_record`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RecordSchema {
    /// Single-writer record with N subkeys (Veilid: DFLT).
    SingleWriter { subkey_count: u32 },
    /// Multi-writer record (Veilid: SMPL).
    ///
    /// `member_keys` carries the Ed25519 public key for each member slot.
    /// Veilid's SMPL schema validation requires the writer's public key to
    /// match the `m_key` in the schema. Pre-derive all 255 keys from the
    /// shared slot_seed via `derive_slot_keypair` at community creation.
    MultiWriter {
        owner_subkeys: u16,
        member_subkeys: u16,
        member_keys: Vec<[u8; 32]>,
    },
}

/// Report from a community broadcast operation.
#[derive(Debug, Clone, Default)]
pub struct BroadcastReport {
    pub peers_sent: u32,
    pub peers_failed: u32,
}

/// Result of a DHT record inspection.
///
/// Contains both local and network sequence numbers so the caller
/// can choose the appropriate view:
/// - `local_seqs`: what this node has in its local cache
/// - `network_seqs`: what remote nodes reported via DHT fanout
///
/// For propagation confirmation, check `network_seqs`.
/// For local catch-up, check `local_seqs`.
#[derive(Debug, Clone, Default)]
pub struct InspectResult {
    /// Sequence numbers from this node's local cache.
    pub local_seqs: Vec<Option<u32>>,
    /// Sequence numbers reported by remote nodes via DHT fanout.
    /// Empty if the inspect was local-only.
    pub network_seqs: Vec<Option<u32>>,
}

/// Proof that a DHT record is open for operations.
///
/// Returned by `create_record` and `open_record`. Required by
/// `write_record`, `read_record`, `watch_record`, `inspect_record`.
/// Consumed by `close_record` (prevents use-after-close).
///
/// Private `key` field — only constructable via `OpenRecord::new()`
/// at the Transport implementation boundary.
#[derive(Debug, Clone)]
pub struct OpenRecord {
    key: String,
}

impl OpenRecord {
    /// Construct from a record key string. Called exclusively by
    /// Transport impls in `create_record` and `open_record`.
    pub fn new(key: String) -> Self {
        Self { key }
    }

    /// The DHT record key string.
    pub fn key(&self) -> &str {
        &self.key
    }
}

/// Opaque token returned by `Transport::watch_record`.
/// Used to cancel the watch via `Transport::cancel_watch`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WatchToken(pub u64);

/// Delivery durability preference — controls fallback behavior.
///
/// The caller chooses durability based on whether a DHT write has
/// already been performed (Durable) or not (Ephemeral).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Durability {
    /// DHT write already done by caller. Direct send is a latency
    /// optimization. Send failure is acceptable — DHT covers delivery.
    Durable,
    /// No DHT write. Direct send is the only delivery path. Send failure
    /// means the message is lost.
    Ephemeral,
}

/// Report from a delivery attempt.
#[derive(Debug, Clone)]
pub struct DeliveryReport {
    /// Whether the direct/gossip send succeeded.
    pub sent: bool,
    /// Number of mesh peers reached (for community delivery).
    pub peers_reached: u32,
    /// Error message if send failed and durability was Ephemeral.
    pub error: Option<String>,
}

/// Transfer progress snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferProgress {
    pub transfer_id: [u8; 16],
    pub filename: String,
    pub total_size: u64,
    pub bytes_transferred: u64,
    pub chunks_received: u32,
    pub chunk_count: u32,
    pub direction: TransferDirection,
    pub status: TransferStatus,
    pub elapsed_secs: u64,
    pub throughput_bytes_per_sec: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransferDirection {
    Send,
    Receive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransferStatus {
    Offering,
    Active,
    Completing,
    Completed,
    Cancelled,
    Failed,
}

// ── Transport Trait ───────────────────────────────────────────────

/// Transport-agnostic interface for sending and receiving bytes.
///
/// Implementations: `rekindle-transport-veilid` (Veilid DHT + gossip),
/// future: `rekindle-transport-matrix`, `rekindle-transport-signal`.
///
/// `rekindle-chat` programs exclusively against this trait. Swapping
/// transport means providing a different implementation — zero chat
/// code changes.
///
/// Inbound data is NOT delivered via this trait. It flows through
/// `mpsc::Sender<InboundEvent>` returned alongside the transport at
/// construction time. The chat layer reads from the receiver.
#[async_trait]
pub trait Transport: Send + Sync + 'static {
    // ── Lifecycle ───────────────────────────────────────────

    /// Start the transport (attach to network). Some backends start at
    /// construction time and this is a no-op.
    async fn start(&self) -> TransportResult<()>;

    /// Graceful shutdown: stop background tasks, release routes, detach.
    async fn shutdown(&self) -> TransportResult<()>;

    /// Whether the transport is attached to the network and ready.
    fn is_attached(&self) -> bool;

    // ── Peer messaging (opaque bytes) ──────────────────────
    async fn send_to_peer(&self, peer_key: &str, data: &[u8]) -> TransportResult<()>;
    async fn call_peer(&self, peer_key: &str, data: &[u8]) -> TransportResult<Vec<u8>>;

    // ── Persistent records ─────────────────────────────────
    async fn create_record(&self, schema: RecordSchema) -> TransportResult<(OpenRecord, Vec<u8>)>;
    async fn open_record(&self, key: &str, writer: Option<&[u8]>) -> TransportResult<OpenRecord>;
    async fn write_record(
        &self, record: &OpenRecord, subkey: u32, data: &[u8], writer: Option<&[u8]>,
    ) -> TransportResult<()>;
    async fn read_record(
        &self, record: &OpenRecord, subkey: u32, force_refresh: bool,
    ) -> TransportResult<Option<Vec<u8>>>;
    async fn watch_record(&self, record: &OpenRecord, subkeys: &[u32]) -> TransportResult<WatchToken>;
    async fn cancel_watch(&self, token: WatchToken) -> TransportResult<()>;
    async fn inspect_record(
        &self, record: &OpenRecord, subkeys: &[u32],
    ) -> TransportResult<InspectResult>;
    async fn close_record(&self, record: OpenRecord) -> TransportResult<()>;

    /// Delete a local copy of a DHT record. Record must be closed first.
    /// Does not delete from the network -- stops local republishing.
    async fn delete_record(&self, key: &str) -> TransportResult<()> {
        Err(TransportError::Internal(format!("delete_record not implemented for {key}")))
    }

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

    /// Add or update a peer in a community's gossip mesh.
    fn upsert_mesh_peer(
        &self, _community_id: &str, _pseudonym: &str,
        _route_blob: Vec<u8>, _status: &str, _now_secs: u64,
        _my_pseudonym: &str,
    ) {}

    /// Remove a peer from a community's gossip mesh.
    fn remove_mesh_peer(&self, _community_id: &str, _pseudonym: &str) {}

    // ── Commoditized delivery ─────────────────────────────

    /// Register a peer→profile_dht_key mapping for route resolution.
    fn register_peer_profile(&self, _peer_key: &str, _profile_dht_key: &str) {}

    /// Deliver a message to a specific peer with durability preference.
    async fn deliver(
        &self, _peer_key: &str, _data: &[u8], _durability: Durability,
    ) -> DeliveryReport {
        DeliveryReport { sent: false, peers_reached: 0, error: Some("not implemented".into()) }
    }

    /// Deliver a message to all peers in a community's gossip mesh.
    async fn deliver_community(
        &self, _community_id: &str, _data: &[u8], _durability: Durability,
    ) -> DeliveryReport {
        DeliveryReport { sent: false, peers_reached: 0, error: Some("not implemented".into()) }
    }

    /// Populate a community's gossip mesh from member data.
    async fn populate_mesh(
        &self, _community_id: &str, _my_pseudonym: &str,
        _members: &[(String, Option<String>)],
    ) -> u32 { 0 }

    /// Refresh routes for all members in a community's gossip mesh.
    async fn refresh_mesh(&self, _community_id: &str, _my_pseudonym: &str) {}

    /// Evict stale members from all gossip meshes.
    fn evict_stale_mesh_members(&self) {}

    // ── Bulk file transfer ────────────────────────────────

    /// Send a file to a peer. Returns the transfer_id for tracking.
    /// Progress events flow through the inbound channel.
    async fn send_file(
        &self, _peer_key: &str, _path: &std::path::Path, _media_type: &str,
    ) -> TransportResult<[u8; 16]> {
        Err(TransportError::Internal("bulk transfer not implemented".into()))
    }

    /// Cancel an active transfer (inbound or outbound).
    fn cancel_transfer(&self, _transfer_id: &[u8; 16]) {}

    /// Get progress for a specific transfer.
    fn get_transfer_progress(&self, _transfer_id: &[u8; 16]) -> Option<TransferProgress> { None }

    /// Get progress for all active transfers.
    fn active_transfers(&self) -> Vec<TransferProgress> { Vec::new() }

    // ── Diagnostics ────────────────────────────────────────
    fn peer_count(&self) -> u32;
    fn attachment_state(&self) -> &str;
    fn uptime_secs(&self) -> u64;

    /// Whether the public internet is reachable via this node.
    fn is_public_internet_ready(&self) -> bool { false }

    /// Seconds since the current personal route was allocated.
    fn route_age_secs(&self) -> Option<u64> { None }

    /// Circuit breaker summary across all known peers.
    /// Returns (total, healthy, degraded, circuit_open) counts.
    fn circuit_summary(&self) -> (usize, usize, usize, usize) { (0, 0, 0, 0) }

    /// Number of peers across all gossip meshes.
    fn gossip_mesh_peer_count(&self) -> usize { 0 }
}
