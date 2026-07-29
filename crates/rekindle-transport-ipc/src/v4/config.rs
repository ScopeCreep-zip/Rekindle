//! Crate-level configuration types for IPC transport sessions.
//!
//! These are construction-time parameters, not runtime state.
//! They are Clone + Debug and passed by value to session setup.

use std::sync::Arc;

use crate::v4::audit::checkpoint::CheckpointConfig;
use crate::v4::audit::retention::RetentionConfig;
use crate::v4::bulk::counters::BulkCounters;
use crate::v4::dedup::cache::{ReceiverCacheConfig, SenderCacheConfig};
use crate::v4::session::handshake::HandshakeConfig;
use crate::v4::stream::resume::ResumeConfig;
use crate::v4::wire::capability::CapabilityBits;
use crate::v4::wire::clearance::Clearance;

/// All tunables for a session. Every subsystem reads from this at
/// construction time. Immutable after session setup.
#[derive(Debug, Clone)]
pub struct SessionConfig {
    pub checkpoint_config: CheckpointConfig,
    pub retention_config: RetentionConfig,
    /// Maximum time (ms) the inbound reorder buffer can stall before
    /// declaring a gap and emitting AUDIT_GAP. Rayon completion reordering
    /// resolves in milliseconds — 500ms means the frame is genuinely lost.
    pub max_reorder_stall_ms: Option<u64>,
    pub initial_stream_credit_chunks: u32,
    pub initial_lane_credit_bytes: u32,
    pub resume_config: ResumeConfig,
    pub sender_cache_config: SenderCacheConfig,
    pub receiver_cache_config: ReceiverCacheConfig,
    /// Bytes per arena slot. Page-aligned upward. Default: 16 MiB.
    /// Must fit in u32 for the ArenaSetup wire format.
    pub arena_slot_size: usize,
    /// Number of arena slots. Default: 8.
    /// Must fit in u16 for the ArenaSetup wire format.
    pub arena_slot_count: usize,
    /// Whether to compute BLAKE3 on publish / verify on read.
    pub arena_integrity_check: bool,
    pub heartbeat_interval_ms: u64,
    pub heartbeat_miss_limit: u32,
    pub heartbeat_response_timeout_ms: u64,
    pub max_pending_requests: usize,
    pub max_subscriptions: usize,
    pub max_pending_bytes_per_session: u64,
    pub max_connections: Option<u32>,
    /// Rayon pool worker count. 0 = auto-detect.
    pub encrypt_workers: Option<usize>,
    /// Inbound Data lane frames >= this threshold go to rayon bulk decrypt.
    pub bulk_decrypt_threshold: Option<u32>,
    /// Reassembler window size. Must be power of two.
    pub reassembler_window: usize,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            checkpoint_config: CheckpointConfig::default(),
            retention_config: RetentionConfig::default(),
            max_reorder_stall_ms: Some(500),
            initial_stream_credit_chunks: 64,
            initial_lane_credit_bytes: 1_048_576,
            resume_config: ResumeConfig::default(),
            sender_cache_config: SenderCacheConfig::default(),
            receiver_cache_config: ReceiverCacheConfig::default(),
            arena_slot_size: 16 * 1024 * 1024,
            arena_slot_count: 8,
            arena_integrity_check: true,
            heartbeat_interval_ms: 15_000,
            heartbeat_miss_limit: 3,
            heartbeat_response_timeout_ms: 5_000,
            max_pending_requests: 4096,
            max_subscriptions: 256,
            max_pending_bytes_per_session: 1_073_741_824,
            max_connections: None,
            encrypt_workers: None,
            bulk_decrypt_threshold: Some(4096),
            reassembler_window: 256,
        }
    }
}

/// Server construction config.
pub struct ServerConfig {
    pub session: SessionConfig,
    pub handshake: HandshakeConfig,
    pub counters: Arc<BulkCounters>,
}

impl ServerConfig {
    pub fn new() -> Self {
        Self {
            session: SessionConfig::default(),
            handshake: HandshakeConfig::default(),
            counters: BulkCounters::new(),
        }
    }

    pub fn for_test() -> Self {
        let mut session = SessionConfig::default();
        session.heartbeat_interval_ms = 1_000;
        session.heartbeat_miss_limit = 3;
        session.heartbeat_response_timeout_ms = 500;
        session.max_connections = Some(16);
        Self {
            session,
            handshake: HandshakeConfig::new(
                CapabilityBits::MANDATORY_V1 | CapabilityBits::AEAD_AEGIS128L,
                Clearance::Internal,
            ),
            counters: BulkCounters::new(),
        }
    }

    pub fn for_bench() -> Self {
        let mut session = SessionConfig::default();
        session.heartbeat_interval_ms = 60_000;
        session.heartbeat_miss_limit = 100;
        session.heartbeat_response_timeout_ms = 60_000;
        session.max_connections = Some(65536);
        Self {
            session,
            handshake: HandshakeConfig::new(
                CapabilityBits::MANDATORY_V1 | CapabilityBits::AEAD_AEGIS128L,
                Clearance::Internal,
            ),
            counters: BulkCounters::new(),
        }
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self::new()
    }
}
