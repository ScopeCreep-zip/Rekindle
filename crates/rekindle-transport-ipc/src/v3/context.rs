//! SessionContext — owns all per-Session subsystem state.
//!
//! Handlers receive `&mut SessionContext` and call into the subsystems.
//! They push `OutboundFrame` entries; the connection driver drains and
//! writes them to the socket after dispatch returns.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;

use crate::v3::audit::chain::AuditChain;
use crate::v3::audit::checkpoint::{CheckpointConfig, CheckpointTracker};
use crate::v3::audit::gap::GapDetector;
use crate::v3::audit::retention::{RetentionBuffer, RetentionConfig};
use crate::v3::bulk::counters::BulkCounters;
use crate::v3::session::handshake::HandshakeConfig;

use crate::v3::crypto::keys::DerivedKeys;
use crate::v3::dedup::cache::{ReceiverCache, ReceiverCacheConfig, SenderCache, SenderCacheConfig};
use crate::v3::handoff::fallback::{FallbackConfig, FallbackTracker};
use crate::v3::router::{ConnectionInfo, FrameRouter};
use crate::v3::session::handshake::HandshakeResult;
use crate::v3::session::rotation::RotationCoordinator;
use crate::v3::session::state::SessionState;
use crate::v3::stream::flow_control::{BackpressureState, CreditTracker};
use crate::v3::stream::reassembler::Reassembler;
use crate::v3::stream::registry::{Direction, RegistryError, StreamRegistry};
use crate::v3::stream::resume::{ResumeConfig, ResumeRegistry, ResumeState};
use crate::v3::stream::state::{StreamEvent, StreamState};
use crate::v3::wire::capability::CapabilityBits;
use crate::v3::wire::clearance::Clearance;
use crate::v3::io::encode::EpochKeys;
use crate::v3::io::epoch_signal::EpochSignal;
use crate::v3::io::lane_channels::{BulkFrame, PlaintextBuf};
use crate::v3::wire::frame_kind::{
    AuditKind, ChannelKind, DatagramKind, HandoffKind, StreamKind,
};

// Re-export wire types that handlers need via OutboundFrame construction,
// so handlers import from context instead of from wire directly.
// This is the layer boundary: handlers import from context, not from wire.
pub use crate::v3::wire::frame_kind::StreamKind as OutboundStreamKind;
pub use crate::v3::wire::frame_kind::ChannelKind as OutboundChannelKind;
pub use crate::v3::wire::frame_kind::DatagramKind as OutboundDatagramKind;
pub use crate::v3::wire::frame_kind::AuditKind as OutboundAuditKind;
pub use crate::v3::wire::frame_kind::HandoffKind as OutboundHandoffKind;
pub use crate::v3::wire::failure::FailureCode as OutboundFailureCode;

// ── OutboundFrame ─────────────────────────────────────────────────

/// A frame queued for outbound delivery. Handlers push these;
/// the connection driver encodes and writes them.
///
/// Typed enums for kind fields — handlers never use raw u8 wire values.
/// The encoder converts to u8 when serializing to wire bytes. This
/// preserves the layer boundary: handlers construct domain-typed frames,
/// the codec/encoder produces wire bytes.
#[derive(Debug)]
pub enum OutboundFrame {
    Channel { kind: ChannelKind, payload: Vec<u8> },
    Datagram { kind: DatagramKind, payload: Vec<u8> },
    Data { stream_id: u8, kind: StreamKind, chunk_index: u32, payload: Vec<u8> },
    Audit { kind: AuditKind, payload: Vec<u8> },
    Handoff { kind: HandoffKind, payload: Vec<u8> },
}

/// A sequenced outbound frame — the sender awaits confirmation that
/// the control loop processed it (encoded + sent to lane_channels)
/// before proceeding with dependent operations.
///
/// Used by BulkSender: STREAM_OPEN must reach the write task before
/// STREAM_PAYLOAD chunks are dispatched to rayon. The oneshot resolves
/// after drain_outbound completes for this frame.
pub struct SequencedOutbound {
    pub frame: OutboundFrame,
    /// Resolved after the frame is encoded and sent to the write task.
    pub confirm: tokio::sync::oneshot::Sender<()>,
    /// Optional second-phase completion. For operations with a two-phase
    /// lifecycle (send + peer response), this is stored in SessionContext
    /// by drain_outbound and resolved by the response handler.
    /// Used by: key rotation (resolved by handle_commit after ROTATE_COMMIT).
    pub completion: Option<tokio::sync::oneshot::Sender<Result<(), crate::v3::session::rotation::RotationError>>>,
}

impl OutboundFrame {
    pub fn lane(&self) -> crate::v3::wire::lane::Lane {
        use crate::v3::wire::lane::Lane;
        match self {
            Self::Channel { .. } | Self::Datagram { .. } => Lane::Control,
            Self::Data { .. } => Lane::Data,
            Self::Audit { .. } => Lane::Audit,
            Self::Handoff { .. } => Lane::Handoff,
        }
    }
}

// ── ServerConfig ─────────────────────────────────────────────────

/// Server construction config. 3 mandatory positional params on bind()
/// (path, keypair, router_factory) + 1 ServerConfig for everything
/// with a sensible default. Adding a field is non-breaking — callers
/// use struct update: `ServerConfig { counters: mine, ..ServerConfig::new() }`
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

// ── SessionConfig ─────────────────────────────────────────────────

/// All tunables for a Session. Every subsystem reads from this.
#[derive(Debug, Clone)]
pub struct SessionConfig {
    pub checkpoint_config: CheckpointConfig,
    pub retention_config: RetentionConfig,
    pub max_gap: u64,
    pub initial_stream_credit_chunks: u32,
    pub initial_lane_credit_bytes: u32,
    pub resume_config: ResumeConfig,
    pub sender_cache_config: SenderCacheConfig,
    pub receiver_cache_config: ReceiverCacheConfig,
    pub handoff_threshold_bytes: u64,
    pub fallback_config: FallbackConfig,
    pub initial_sidechannel_credit: u16,
    pub heartbeat_interval_ms: u64,
    pub heartbeat_miss_limit: u32,
    pub heartbeat_response_timeout_ms: u64,
    pub max_pending_requests: usize,
    pub max_subscriptions: usize,
    pub max_pending_bytes_per_session: u64,
    pub max_connections: Option<u32>,
    /// Rayon encryption pool worker count. 0 = auto-detect (physical_cores - 2).
    /// Set to constrain on large machines (e.g., 2 on a 92-core server).
    pub encrypt_workers: Option<usize>,
    /// Inbound Data lane frames with body_len >= this threshold are dispatched
    /// to the rayon pool for parallel AEAD decryption via BulkReceiver.
    /// Frames below this threshold are decrypted inline by the read task.
    /// Default: 65536 (64 KiB). Set to u32::MAX to disable parallel decrypt.
    pub bulk_decrypt_threshold: Option<u32>,
    /// ReorderRing window for the per-stream chunk reassembler.
    /// Must be a power of two. Default 256 (supports up to 256
    /// outstanding out-of-order chunks per stream, ~4 GiB at 16 MiB/chunk).
    pub reassembler_window: usize,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            checkpoint_config: CheckpointConfig::default(),
            retention_config: RetentionConfig::default(),
            max_gap: 65536,
            initial_stream_credit_chunks: 64,
            initial_lane_credit_bytes: 1_048_576,
            resume_config: ResumeConfig::default(),
            sender_cache_config: SenderCacheConfig::default(),
            receiver_cache_config: ReceiverCacheConfig::default(),
            handoff_threshold_bytes: 262_144,
            fallback_config: FallbackConfig::default(),
            initial_sidechannel_credit: 8,
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

// ── Internal types ────────────────────────────────────────────────

pub(crate) struct PendingRequest {
    sent_at: Instant,
    timeout_ms: u32,
}

impl PendingRequest {
    pub fn is_expired(&self, now: Instant) -> bool {
        now.duration_since(self.sent_at).as_millis() as u32 >= self.timeout_ms
    }
}

struct Subscription {
    topics: Vec<[u8; 32]>,
    conditions: Vec<u8>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct NonceExhausted;

pub struct VerifiedProof {
    pub query_id: uuid::Uuid,
    pub session_seq: u64,
    pub link: [u8; 32],
    pub anchor_link: [u8; 32],
    pub verified: bool,
}

// ── Bulk FIN tracking ─────────────────────────────────────────────

/// Sent by BulkSender to the control loop after spawning all rayon workers.
/// The control loop counts arriving OutboundAuditLinks and emits STREAM_FIN
/// through bulk_wire_tx when all payload LinkInputs have been processed.
pub struct PendingFin {
    pub stream_id: u8,
    pub chunk_count: u32,
    pub total_bytes: u64,
    pub content_hash: [u8; 32],
    pub bulk_wire_tx: crossbeam::channel::Sender<BulkFrame>,
    /// The session_seq of the last payload chunk. The outbound audit
    /// reorder buffer must advance past this seq before FIN can be emitted.
    pub last_chunk_seq: u64,
}

/// Control loop state for a pending FIN emission.
pub struct PendingFinState {
    pub chunk_count: u32,
    pub total_bytes: u64,
    pub content_hash: [u8; 32],
    pub bulk_wire_tx: crossbeam::channel::Sender<BulkFrame>,
    /// The session_seq of the last payload chunk. FIN is emitted when
    /// outbound_reorder.next_expected() > last_chunk_seq — meaning all
    /// payload LinkInputs have been flushed through the audit chain.
    pub last_chunk_seq: u64,
}

/// Receive-side: stored when a FIN chunk arrives before all payload chunks.
/// Verification is deferred until the reassembler has all chunks.
pub struct PendingFinVerify {
    pub content_hash: [u8; 32],
    pub audit_link: [u8; 32],
    pub total_bytes: u64,
    pub transfer_id: uuid::Uuid,
    /// Total payload chunk count (indices 0..expected_chunks-1).
    /// Equals the FIN frame's chunk_index field. The reassembler is
    /// complete when next_expected >= expected_chunks && buffered == 0.
    pub expected_chunks: u32,
}

// ── SessionContext ────────────────────────────────────────────────

pub struct SessionContext {
    // Identity
    session_id: uuid::Uuid,
    local_peer_id: [u8; 32],
    remote_peer_id: [u8; 32],
    agreed_clearance: Clearance,
    active_capabilities: CapabilityBits,

    // Router — delivers application-visible content
    router: Arc<dyn FrameRouter>,
    connection_info: ConnectionInfo,
    conn_id: u64,

    // Crypto
    keys: DerivedKeys,
    rotation: RotationCoordinator,

    // Session lifecycle
    session_state: SessionState,
    send_seq: u64,
    recv_last_seq: u64,

    // Audit
    outbound_chain: AuditChain,
    inbound_chain: AuditChain,
    outbound_checkpoint: CheckpointTracker,
    inbound_checkpoint: CheckpointTracker,
    gap_detector: GapDetector,
    retention: RetentionBuffer,

    // Stream
    stream_registry: StreamRegistry,
    reassemblers: HashMap<u8, Reassembler>,
    stream_credits: HashMap<u8, CreditTracker>,
    lane_credit_bytes: HashMap<u8, u32>,
    backpressure: BackpressureState,
    // Per-stream mapping: (stream_id, chunk_index) → session_seq
    // Populated when sending STREAM_PAYLOAD, used by SACK to release retention
    chunk_to_seq: HashMap<(u8, u32), u64>,

    // Resume
    resume_registry: ResumeRegistry,

    // Dedup
    sender_cache: SenderCache,
    receiver_cache: ReceiverCache,

    // Handoff
    fallback_tracker: FallbackTracker,
    pending_handoffs: HashSet<uuid::Uuid>,

    // Datagram
    pending_requests: HashMap<uuid::Uuid, PendingRequest>,
    config: SessionConfig,

    // Subscription
    subscriptions: HashMap<uuid::Uuid, Subscription>,
    topic_index: HashMap<[u8; 32], Vec<uuid::Uuid>>,

    // Heartbeat — two-generation nonce model (custom extension;
    // SCTP §8.3 uses single nonce with silent discard on mismatch).
    // current: the nonce of the most recently sent PING.
    // previous: the nonce of the PING before that (rotated on timeout).
    // A PONG matching either generation is valid — the peer is alive.
    last_ping_nonce: Option<u64>,
    previous_ping_nonce: Option<u64>,
    last_pong_received: Option<Instant>,
    heartbeat_miss_count: u32,
    remote_last_seen_our_seq: u64,

    // Audit proofs
    verified_proofs: HashMap<uuid::Uuid, VerifiedProof>,

    // Goodbye
    peer_final_session_seq: Option<u64>,
    local_goodbye_sent: bool,

    // Deadlines
    quiescence_deadline: Option<Instant>,
    rotation_deadline: Option<Instant>,
    drain_deadline: Option<Instant>,

    // Bulk FIN tracking — send side
    pending_fins: HashMap<u8, PendingFinState>,
    // Bulk FIN tracking — receive side (deferred verification)
    pending_fin_verify: HashMap<u8, PendingFinVerify>,

    // Output
    outbound: Vec<OutboundFrame>,

    // Bulk data delivery — chunks staged by handlers, drained by control loop.
    // Handlers push (stream_id, chunk_index, Vec<u8>) here after reassembler
    // delivers. The control loop sends them through bulk_data_tx to the client.
    // Same pattern as `outbound` for control-plane frames.
    pending_bulk_deliveries: Vec<(u8, u32, PlaintextBuf)>,
    /// Bulk transfer completions staged by handlers (on_bulk_complete).
    /// Drained by the control loop AFTER pending_bulk_deliveries so the
    /// completion signal arrives at the bridge task AFTER all data chunks.
    pending_bulk_completions: Vec<(u8, uuid::Uuid, u64, u32)>, // (stream_id, transfer_id, total_bytes, total_chunks)

    // Early bulk chunks — buffered when BulkDecrypted arrives before
    // STREAM_OPEN creates the reassembler. The rayon decrypt path can
    // deliver chunks faster than the inline signal bridge delivers
    // STREAM_OPEN. Replayed into the reassembler when create_reassembler
    // is called. Bounded: max 64 early chunks per stream to prevent
    // unbounded growth from a missing STREAM_OPEN.
    early_bulk_chunks: HashMap<u8, Vec<(u32, PlaintextBuf, [u8; 32])>>,

    // Key rotation completion — oneshot resolved by handle_commit after
    // both sides derive matching new keys.
    pending_rotation_confirm: Option<tokio::sync::oneshot::Sender<Result<(), crate::v3::session::rotation::RotationError>>>,

    // Session role — determines key direction mapping.
    role: SessionRole,

    // AEAD algorithm code negotiated at handshake.
    agreed_aead: u8,

    // Epoch-tagged key rotation state.
    // signal_epoch tracks the epoch for EpochSignal computation.
    // Incremented by install_next_epoch_keys (control loop thread).
    // Independent of the encoder's epoch (read task thread).
    // Each rotation targets signal_epoch ^ 1, then signal_epoch is updated.
    signal_epoch: u8,
    epoch_signal: Arc<EpochSignal>,
    // Encoder key installation handled by read task via EpochSignal.
    // No pending encoder storage needed.
    // No epoch_retirement_deadline — retirement is slot overwrite, not timer.

    // Handoff coordinator — manages memfd lifecycle for zero-kernel-copy
    // transfers above handoff_threshold_bytes. Platform-gated: Linux only.
    // None when HANDOFF_MEMFD capability is not negotiated.
    #[cfg(target_os = "linux")]
    handoff_coordinator: Option<crate::v3::handoff::coordinator::HandoffCoordinator<
        crate::v3::handoff::transport::LinuxTransport,
        crate::v3::handoff::transport::LinuxMemfd,
    >>,
}

/// Role of this peer in the session — determines key direction mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionRole {
    /// We are the dialler (initiator). Outbound = d2l, inbound = l2d.
    Dialler,
    /// We are the listener (responder). Outbound = l2d, inbound = d2l.
    Listener,
}

impl SessionContext {
    pub fn from_handshake(
        result: HandshakeResult,
        config: SessionConfig,
        router: Arc<dyn FrameRouter>,
        conn_id: u64,
        role: SessionRole,
        agreed_aead: u8,
    ) -> Self {
        let (outbound_audit_key, inbound_audit_key) = match role {
            SessionRole::Dialler => (result.keys.audit_d2l, result.keys.audit_l2d),
            SessionRole::Listener => (result.keys.audit_l2d, result.keys.audit_d2l),
        };
        let outbound_chain = AuditChain::new(outbound_audit_key, result.handshake_hash);
        let inbound_chain = AuditChain::new(inbound_audit_key, result.handshake_hash);
        let rotation = RotationCoordinator::new(result.keys.clone(), 0);

        let connection_info = ConnectionInfo {
            conn_id,
            session_id: result.session_id,
            peer_id: result.remote_peer_id,
            clearance: result.agreed_clearance,
            capabilities: result.active_capabilities,
        };

        Self {
            session_id: result.session_id,
            local_peer_id: result.local_peer_id,
            remote_peer_id: result.remote_peer_id,
            agreed_clearance: result.agreed_clearance,
            active_capabilities: result.active_capabilities,
            router,
            connection_info,
            conn_id,
            keys: result.keys,
            rotation,
            session_state: SessionState::Established,
            send_seq: 0,
            recv_last_seq: 0,
            outbound_chain,
            inbound_chain,
            outbound_checkpoint: CheckpointTracker::new(config.checkpoint_config),
            inbound_checkpoint: CheckpointTracker::new(config.checkpoint_config),
            gap_detector: GapDetector::with_max_gap(config.max_gap),
            retention: RetentionBuffer::new(config.retention_config.clone()),
            stream_registry: StreamRegistry::new(),
            reassemblers: HashMap::new(),
            stream_credits: HashMap::new(),
            lane_credit_bytes: HashMap::new(),
            backpressure: BackpressureState::new(),
            chunk_to_seq: HashMap::new(),
            resume_registry: ResumeRegistry::new(),
            sender_cache: SenderCache::new(config.sender_cache_config.clone()),
            receiver_cache: ReceiverCache::new(config.receiver_cache_config.clone()),
            fallback_tracker: FallbackTracker::new(config.fallback_config.clone()),
            pending_handoffs: HashSet::new(),
            pending_requests: HashMap::new(),
            subscriptions: HashMap::new(),
            topic_index: HashMap::new(),
            last_ping_nonce: None,
            previous_ping_nonce: None,
            last_pong_received: None,
            heartbeat_miss_count: 0,
            remote_last_seen_our_seq: 0,
            verified_proofs: HashMap::new(),
            peer_final_session_seq: None,
            local_goodbye_sent: false,
            quiescence_deadline: None,
            rotation_deadline: None,
            drain_deadline: None,
            pending_fins: HashMap::new(),
            pending_fin_verify: HashMap::new(),
            outbound: Vec::new(),
            pending_bulk_deliveries: Vec::new(),
            pending_bulk_completions: Vec::new(),
            early_bulk_chunks: HashMap::new(),
            pending_rotation_confirm: None,
            role,
            agreed_aead,
            signal_epoch: 0,
            epoch_signal: Arc::new(EpochSignal::new()),
                    #[cfg(target_os = "linux")]
            handoff_coordinator: None,
            config,
        }
    }

    // ── Identity accessors ────────────────────────────────────────

    pub fn session_id(&self) -> uuid::Uuid { self.session_id }
    pub fn local_peer_id(&self) -> &[u8; 32] { &self.local_peer_id }
    pub fn remote_peer_id(&self) -> &[u8; 32] { &self.remote_peer_id }
    pub fn agreed_clearance(&self) -> Clearance { self.agreed_clearance }
    pub fn active_capabilities(&self) -> CapabilityBits { self.active_capabilities }
    pub fn has_resume(&self) -> bool { self.active_capabilities.contains(CapabilityBits::RESUME) }
    pub fn has_dedup(&self) -> bool { self.active_capabilities.contains(CapabilityBits::DEDUP_CACHE) }
    pub fn has_handoff(&self) -> bool { self.active_capabilities.contains(CapabilityBits::HANDOFF_MEMFD) }
    pub fn has_subscription(&self) -> bool { self.active_capabilities.contains(CapabilityBits::SUBSCRIPTION) }

    /// Build a CHANNEL_ERROR outbound frame from an error code and message.
    pub fn push_channel_error(&mut self, error_code: u16, message: &str) {
        use crate::v3::codec::channel::error as error_codec;
        let payload = error_codec::encode(&error_codec::ErrorPayload {
            error_code,
            message: message.to_owned(),
        });
        self.push_outbound(OutboundFrame::Channel {
            kind: ChannelKind::ChannelError,
            payload,
        });
    }

    // ── Keys ──────────────────────────────────────────────────────

    pub fn keys(&self) -> &DerivedKeys { &self.keys }
    pub fn set_keys(&mut self, keys: DerivedKeys) { self.keys = keys; }
    pub fn rotation_mut(&mut self) -> &mut RotationCoordinator { &mut self.rotation }
    pub fn role(&self) -> SessionRole { self.role }
    pub fn agreed_aead(&self) -> u8 { self.agreed_aead }

    // ── Epoch-tagged key rotation ────────────────────────────
    //
    // The encoder (FrameEncoder::EncoderState::epoch) is the SSOT for the
    // current epoch number. SessionContext only holds the EpochSignal
    // (for decoder key installation) and the two pending encoder key slots
    // (immediate for initiator, deferred for responder).

    /// Signal-side epoch — tracks the epoch for EpochSignal computation.
    /// NOT the encoder's current epoch (which is on the read task thread).
    pub fn signal_epoch(&self) -> u8 { self.signal_epoch }

    /// Install both decoder and encoder keys for the next epoch.
    /// Computes the epoch slot from signal_epoch ^ 1 and sends both
    /// key sets via the epoch signal. The read task installs both
    /// atomically before reading the next frame — no cross-thread race.
    pub fn install_next_epoch_keys(&mut self, decoder_keys: EpochKeys, encoder_keys: EpochKeys) {
        let next_epoch = self.signal_epoch ^ 1;
        self.signal_epoch = next_epoch;
        tracing::debug!(
            signal_epoch = self.signal_epoch,
            next_epoch,
            dec_key_fp = %hex::encode(&decoder_keys.envelope_key[..8]),
            enc_key_fp = %hex::encode(&encoder_keys.envelope_key[..8]),
            "SessionContext::install_next_epoch_keys"
        );
        self.epoch_signal.send_install(next_epoch, decoder_keys, encoder_keys);
    }

    // Retirement is slot overwrite — no explicit signal needed.
    // When install(epoch=N) fires, key_slots[N&1] drops the previous
    // occupant (epoch=N-2). ZeroizeOnDrop fires on the dropped keys.

    pub fn epoch_signal(&self) -> &Arc<EpochSignal> { &self.epoch_signal }

    // Encoder key installation is handled by the read task via EpochSignal
    // (carries both decoder + encoder keys). No immediate/deferred storage
    // needed in SessionContext. Retirement is slot overwrite.

    // ── Session state ─────────────────────────────────────────────

    pub fn session_state(&self) -> SessionState { self.session_state }
    pub fn set_session_state(&mut self, state: SessionState) { self.session_state = state; }
    pub fn session_state_mut(&mut self) -> &mut SessionState { &mut self.session_state }
    pub fn send_seq(&self) -> u64 { self.send_seq }
    pub fn recv_last_seq(&self) -> u64 { self.recv_last_seq }

    /// Allocate the next outbound session_seq. Returns Err if the nonce
    /// space is exhausted (u64::MAX - 1 reached; u64::MAX is reserved
    /// for Noise rekey). The Session must be closed on exhaustion.
    pub fn next_send_seq(&mut self) -> Result<u64, NonceExhausted> {
        if self.send_seq >= u64::MAX - 1 {
            return Err(NonceExhausted);
        }
        let seq = self.send_seq;
        self.send_seq += 1;
        Ok(seq)
    }

    /// Record the last verified inbound session_seq.
    pub fn advance_recv_seq(&mut self, seq: u64) {
        self.recv_last_seq = seq;
    }

    // ── Deadlines ─────────────────────────────────────────────────

    /// Set the quiescence deadline. The driver checks this on every iteration.
    pub fn set_quiescence_deadline(&mut self, deadline: Instant) {
        self.quiescence_deadline = Some(deadline);
    }

    pub fn clear_quiescence_deadline(&mut self) {
        self.quiescence_deadline = None;
    }

    pub fn quiescence_deadline(&self) -> Option<Instant> {
        self.quiescence_deadline
    }

    /// Set the rotation deadline. The driver checks this on every iteration.
    pub fn set_rotation_deadline(&mut self, deadline: Instant) {
        self.rotation_deadline = Some(deadline);
    }

    pub fn clear_rotation_deadline(&mut self) {
        self.rotation_deadline = None;
    }

    pub fn rotation_deadline(&self) -> Option<Instant> {
        self.rotation_deadline
    }

    pub fn set_drain_deadline(&mut self, deadline: Instant) {
        self.drain_deadline = Some(deadline);
    }

    pub fn drain_deadline(&self) -> Option<Instant> {
        self.drain_deadline
    }

    // ── Audit chain ───────────────────────────────────────────────

    pub fn outbound_chain(&self) -> &AuditChain { &self.outbound_chain }
    pub fn outbound_chain_mut(&mut self) -> &mut AuditChain { &mut self.outbound_chain }
    pub fn inbound_chain(&self) -> &AuditChain { &self.inbound_chain }
    pub fn inbound_chain_mut(&mut self) -> &mut AuditChain { &mut self.inbound_chain }
    pub fn inbound_checkpoint_mut(&mut self) -> &mut CheckpointTracker { &mut self.inbound_checkpoint }
    pub fn retention(&self) -> &RetentionBuffer { &self.retention }
    pub fn retention_mut(&mut self) -> &mut RetentionBuffer { &mut self.retention }
    pub fn outbound_checkpoint_mut(&mut self) -> &mut CheckpointTracker { &mut self.outbound_checkpoint }
    pub fn gap_detector_mut(&mut self) -> &mut GapDetector { &mut self.gap_detector }

    pub fn store_verified_proof(&mut self, proof: VerifiedProof) {
        self.verified_proofs.insert(proof.query_id, proof);
    }

    pub fn take_verified_proof(&mut self, query_id: &uuid::Uuid) -> Option<VerifiedProof> {
        self.verified_proofs.remove(query_id)
    }

    // ── Stream ────────────────────────────────────────────────────

    // ── Direction-aware stream lifecycle ──────────────────────────

    pub fn open_inbound_stream(&mut self, id: u8) -> Result<(), RegistryError> {
        self.stream_registry.open(id, Direction::Inbound)
    }

    pub fn open_outbound_stream(&mut self, id: u8) -> Result<(), RegistryError> {
        self.stream_registry.open(id, Direction::Outbound)
    }

    pub fn close_inbound_stream(&mut self, id: u8) -> Result<(), RegistryError> {
        self.stream_registry.close(id, Direction::Inbound)
    }

    pub fn close_outbound_stream(&mut self, id: u8) -> Result<(), RegistryError> {
        self.stream_registry.close(id, Direction::Outbound)
    }

    pub fn reset_inbound_stream(&mut self, id: u8) -> Result<(), RegistryError> {
        self.stream_registry.reset(id, Direction::Inbound)
    }

    pub fn reset_outbound_stream(&mut self, id: u8) -> Result<(), RegistryError> {
        self.stream_registry.reset(id, Direction::Outbound)
    }

    pub fn inbound_stream_state(&self, id: u8) -> Option<StreamState> {
        self.stream_registry.state(id, Direction::Inbound)
    }

    pub fn outbound_stream_state(&self, id: u8) -> Option<StreamState> {
        self.stream_registry.state(id, Direction::Outbound)
    }

    pub fn transition_inbound_stream(&mut self, id: u8, event: StreamEvent) -> Result<(), RegistryError> {
        self.stream_registry.transition(id, Direction::Inbound, event)
    }

    pub fn transition_outbound_stream(&mut self, id: u8, event: StreamEvent) -> Result<(), RegistryError> {
        self.stream_registry.transition(id, Direction::Outbound, event)
    }

    pub fn stream_active_count(&self) -> usize {
        self.stream_registry.active_count()
    }

    pub fn create_reassembler(&mut self, stream_id: u8) {
        let window = self.config.reassembler_window;
        self.reassemblers.insert(stream_id, Reassembler::new(window));
        self.stream_credits.insert(
            stream_id,
            CreditTracker::new(
                self.config.initial_stream_credit_chunks,
                self.config.initial_lane_credit_bytes,
            ),
        );
    }

    pub fn create_reassembler_from_offset(&mut self, stream_id: u8, start_chunk: u32) {
        let window = self.config.reassembler_window;
        self.reassemblers.insert(stream_id, Reassembler::new_from_offset(window, start_chunk));
        self.stream_credits.insert(
            stream_id,
            CreditTracker::new(
                self.config.initial_stream_credit_chunks,
                self.config.initial_lane_credit_bytes,
            ),
        );
    }

    pub fn has_reassembler(&self, stream_id: u8) -> bool {
        self.reassemblers.contains_key(&stream_id)
    }

    pub fn remove_reassembler(&mut self, stream_id: u8) {
        self.reassemblers.remove(&stream_id);
        self.stream_credits.remove(&stream_id);
    }

    pub fn reassembler_next_expected(&self, stream_id: u8) -> u32 {
        self.reassemblers.get(&stream_id)
            .map_or(0, Reassembler::next_expected)
    }

    pub fn reassembler_buffered_count(&self, stream_id: u8) -> usize {
        self.reassemblers.get(&stream_id)
            .map_or(0, Reassembler::buffered_count)
    }

    pub fn record_chunk_session_seq(&mut self, stream_id: u8, chunk_index: u32, session_seq: u64) {
        self.chunk_to_seq.insert((stream_id, chunk_index), session_seq);
    }

    pub fn chunk_session_seq(&self, stream_id: u8, chunk_index: u32) -> Option<u64> {
        self.chunk_to_seq.get(&(stream_id, chunk_index)).copied()
    }

    pub fn reassembler_mut(&mut self, stream_id: u8) -> Option<&mut Reassembler> {
        self.reassemblers.get_mut(&stream_id)
    }

    pub fn stream_credit_remaining(&self, stream_id: u8) -> u32 {
        self.stream_credits.get(&stream_id)
            .map_or(0, CreditTracker::remaining_chunks)
    }

    pub fn stream_credit_tracker_mut(&mut self, stream_id: u8) -> Option<&mut CreditTracker> {
        self.stream_credits.get_mut(&stream_id)
    }

    pub fn lane_credit_bytes(&self, lane: u8) -> u32 {
        self.lane_credit_bytes.get(&lane).copied().unwrap_or(0)
    }

    pub fn set_lane_credit_bytes(&mut self, lane: u8, bytes: u32) {
        self.lane_credit_bytes.insert(lane, bytes);
    }

    // ── Backpressure ──────────────────────────────────────────────

    pub fn is_backpressured(&self) -> bool { self.backpressure.is_blocked() }
    pub fn backpressure_mut(&mut self) -> &mut BackpressureState { &mut self.backpressure }

    // ── Resume ────────────────────────────────────────────────────

    pub fn register_resume_state(
        &mut self,
        transfer_id: uuid::Uuid,
        byte: u64,
        chunk: u32,
        audit_link: [u8; 32],
        content_hash: [u8; 32],
    ) {
        self.resume_registry.register(ResumeState::new(
            transfer_id, byte, chunk, audit_link, content_hash,
            Instant::now(),
            self.config.resume_config.eligibility_window,
        ));
    }

    pub fn has_resume_state(&self, transfer_id: uuid::Uuid) -> bool {
        self.resume_registry.lookup(transfer_id).is_some()
    }

    pub fn resume_registry(&self) -> &ResumeRegistry { &self.resume_registry }
    pub fn resume_registry_mut(&mut self) -> &mut ResumeRegistry { &mut self.resume_registry }

    // ── Dedup ─────────────────────────────────────────────────────

    pub fn sender_cache(&self) -> &SenderCache { &self.sender_cache }
    pub fn sender_cache_mut(&mut self) -> &mut SenderCache { &mut self.sender_cache }
    pub fn receiver_cache(&self) -> &ReceiverCache { &self.receiver_cache }
    pub fn receiver_cache_mut(&mut self) -> &mut ReceiverCache { &mut self.receiver_cache }

    pub fn remove_content_hash(&mut self, hash: &[u8; 32]) {
        self.sender_cache.remove(hash);
        self.receiver_cache.remove(hash);
    }

    // ── Handoff ───────────────────────────────────────────────────

    pub fn fallback_tracker(&self) -> &FallbackTracker { &self.fallback_tracker }
    pub fn fallback_tracker_mut(&mut self) -> &mut FallbackTracker { &mut self.fallback_tracker }

    pub fn register_pending_handoff(&mut self, handoff_id: uuid::Uuid) {
        self.pending_handoffs.insert(handoff_id);
    }

    pub fn remove_pending_handoff(&mut self, handoff_id: &uuid::Uuid) -> bool {
        self.pending_handoffs.remove(handoff_id)
    }

    // ── Datagram correlation ──────────────────────────────────────

    pub fn register_pending_request(&mut self, message_id: uuid::Uuid, timeout_ms: u32) {
        self.pending_requests.insert(message_id, PendingRequest {
            sent_at: Instant::now(),
            timeout_ms,
        });
    }

    pub fn resolve_pending_request(&mut self, message_id: &uuid::Uuid) -> bool {
        self.pending_requests.remove(message_id).is_some()
    }

    pub fn pending_request_count(&self) -> usize { self.pending_requests.len() }

    pub fn max_pending_requests(&self) -> usize { self.config.max_pending_requests }

    /// Remove all pending requests that have exceeded their timeout_ms.
    /// Returns the message IDs of expired requests so the caller can
    /// surface reply-timeout outcomes.
    pub fn sweep_expired_requests(&mut self) -> Vec<uuid::Uuid> {
        let now = Instant::now();
        let expired: Vec<uuid::Uuid> = self.pending_requests.iter()
            .filter(|(_, req)| req.is_expired(now))
            .map(|(id, _)| *id)
            .collect();
        for id in &expired {
            self.pending_requests.remove(id);
        }
        expired
    }

    // ── Subscription ──────────────────────────────────────────────

    pub fn register_subscription(
        &mut self,
        sub_id: uuid::Uuid,
        topics: &[[u8; 32]],
        conditions: &[u8],
    ) {
        let topic_vec = topics.to_vec();
        for topic in &topic_vec {
            self.topic_index.entry(*topic).or_default().push(sub_id);
        }
        self.subscriptions.insert(sub_id, Subscription {
            topics: topic_vec,
            conditions: conditions.to_vec(),
        });
    }

    pub fn remove_subscription(&mut self, sub_id: &uuid::Uuid) -> bool {
        if let Some(sub) = self.subscriptions.remove(sub_id) {
            for topic in &sub.topics {
                if let Some(subs) = self.topic_index.get_mut(topic) {
                    subs.retain(|id| id != sub_id);
                    if subs.is_empty() {
                        self.topic_index.remove(topic);
                    }
                }
            }
            true
        } else {
            false
        }
    }

    pub fn subscription_count(&self) -> usize { self.subscriptions.len() }
    pub fn max_subscriptions(&self) -> usize { self.config.max_subscriptions }

    pub fn has_subscription_for_topic(&self, topic: &[u8; 32]) -> bool {
        self.topic_index.contains_key(topic)
    }

    pub fn subscriptions_for_topic(&self, topic: &[u8; 32]) -> Vec<uuid::Uuid> {
        self.topic_index.get(topic).cloned().unwrap_or_default()
    }

    pub fn subscription_conditions(&self, sub_id: &uuid::Uuid) -> Option<&[u8]> {
        self.subscriptions.get(sub_id).map(|s| s.conditions.as_slice())
    }

    // ── Heartbeat ─────────────────────────────────────────────────

    pub fn set_last_ping_nonce(&mut self, nonce: u64) { self.last_ping_nonce = Some(nonce); }
    pub fn last_ping_nonce(&self) -> Option<u64> { self.last_ping_nonce }
    pub fn clear_last_ping_nonce(&mut self) { self.last_ping_nonce = None; }
    pub fn previous_ping_nonce(&self) -> Option<u64> { self.previous_ping_nonce }
    /// Rotate nonce generations: current → previous, current = None.
    /// Called by pong timeout — gives in-flight PONGs one more cycle.
    pub fn rotate_ping_nonce(&mut self) {
        self.previous_ping_nonce = self.last_ping_nonce.take();
    }
    /// Clear both nonce generations. Called on successful PONG match.
    pub fn clear_all_ping_nonces(&mut self) {
        self.last_ping_nonce = None;
        self.previous_ping_nonce = None;
    }
    /// Clear only the previous nonce. Called when a stale PONG matches.
    pub fn clear_previous_ping_nonce(&mut self) { self.previous_ping_nonce = None; }
    pub fn heartbeat_miss_count(&self) -> u32 { self.heartbeat_miss_count }
    pub fn increment_heartbeat_miss(&mut self) { self.heartbeat_miss_count += 1; }
    pub fn reset_heartbeat_miss(&mut self) { self.heartbeat_miss_count = 0; }
    pub fn remote_last_seen_our_seq(&self) -> u64 { self.remote_last_seen_our_seq }
    pub fn set_remote_last_seen_our_seq(&mut self, seq: u64) { self.remote_last_seen_our_seq = seq; }
    pub fn record_pong_received(&mut self) {
        self.last_pong_received = Some(Instant::now());
    }
    pub fn last_pong_received(&self) -> Option<Instant> { self.last_pong_received }
    pub fn rtt_estimate(&self) -> Option<std::time::Duration> {
        self.last_pong_received.map(|t| t.elapsed())
    }

    // ── Goodbye ───────────────────────────────────────────────────

    pub fn peer_final_session_seq(&self) -> Option<u64> { self.peer_final_session_seq }
    pub fn set_peer_final_session_seq(&mut self, seq: u64) { self.peer_final_session_seq = Some(seq); }
    pub fn mark_local_goodbye_sent(&mut self) { self.local_goodbye_sent = true; }
    pub fn local_goodbye_sent(&self) -> bool { self.local_goodbye_sent }

    // ── Router ─────────────────────────────────────────────────────

    pub fn router(&self) -> &dyn FrameRouter { self.router.as_ref() }
    pub fn connection_info(&self) -> &ConnectionInfo { &self.connection_info }
    pub fn conn_id(&self) -> u64 { self.conn_id }

    // ── Output queue ──────────────────────────────────────────────

    pub fn push_outbound(&mut self, frame: OutboundFrame) { self.outbound.push(frame); }

    pub fn drain_outbound(&mut self) -> Vec<OutboundFrame> {
        std::mem::take(&mut self.outbound)
    }

    // ── Config ────────────────────────────────────────────────────

    pub fn config(&self) -> &SessionConfig { &self.config }

    /// Construct a SessionContext from individual fields and a role.
    ///
    /// Used when the caller has already extracted encoder/decoder from
    /// HandshakeResult (which is not Clone) and needs to construct the
    /// context from the remaining fields.
    pub fn from_fields(
        session_id: uuid::Uuid,
        local_peer_id: [u8; 32],
        remote_peer_id: [u8; 32],
        agreed_clearance: Clearance,
        active_capabilities: CapabilityBits,
        handshake_hash: [u8; 32],
        keys: DerivedKeys,
        config: SessionConfig,
        router: Arc<dyn FrameRouter>,
        conn_id: u64,
        role: SessionRole,
        agreed_aead: u8,
    ) -> Self {
        let (outbound_audit_key, inbound_audit_key) = match role {
            SessionRole::Dialler => (keys.audit_d2l, keys.audit_l2d),
            SessionRole::Listener => (keys.audit_l2d, keys.audit_d2l),
        };
        let outbound_chain = AuditChain::new(outbound_audit_key, handshake_hash);
        let inbound_chain = AuditChain::new(inbound_audit_key, handshake_hash);
        let rotation = RotationCoordinator::new(keys.clone(), 0);

        let connection_info = ConnectionInfo {
            conn_id,
            session_id,
            peer_id: remote_peer_id,
            clearance: agreed_clearance,
            capabilities: active_capabilities,
        };

        Self {
            session_id,
            local_peer_id,
            remote_peer_id,
            agreed_clearance,
            active_capabilities,
            router,
            connection_info,
            conn_id,
            keys,
            rotation,
            session_state: SessionState::Established,
            send_seq: 0,
            recv_last_seq: 0,
            outbound_chain,
            inbound_chain,
            outbound_checkpoint: CheckpointTracker::new(config.checkpoint_config),
            inbound_checkpoint: CheckpointTracker::new(config.checkpoint_config),
            gap_detector: GapDetector::with_max_gap(config.max_gap),
            retention: RetentionBuffer::new(config.retention_config.clone()),
            stream_registry: StreamRegistry::new(),
            reassemblers: HashMap::new(),
            stream_credits: HashMap::new(),
            lane_credit_bytes: HashMap::new(),
            backpressure: BackpressureState::new(),
            chunk_to_seq: HashMap::new(),
            resume_registry: ResumeRegistry::new(),
            sender_cache: SenderCache::new(config.sender_cache_config.clone()),
            receiver_cache: ReceiverCache::new(config.receiver_cache_config.clone()),
            fallback_tracker: FallbackTracker::new(config.fallback_config.clone()),
            pending_handoffs: HashSet::new(),
            pending_requests: HashMap::new(),
            subscriptions: HashMap::new(),
            topic_index: HashMap::new(),
            last_ping_nonce: None,
            previous_ping_nonce: None,
            last_pong_received: None,
            heartbeat_miss_count: 0,
            remote_last_seen_our_seq: 0,
            verified_proofs: HashMap::new(),
            peer_final_session_seq: None,
            local_goodbye_sent: false,
            quiescence_deadline: None,
            rotation_deadline: None,
            drain_deadline: None,
            pending_fins: HashMap::new(),
            pending_fin_verify: HashMap::new(),
            outbound: Vec::new(),
            pending_bulk_deliveries: Vec::new(),
            pending_bulk_completions: Vec::new(),
            early_bulk_chunks: HashMap::new(),
            pending_rotation_confirm: None,
            role,
            agreed_aead,
            signal_epoch: 0,
            epoch_signal: Arc::new(EpochSignal::new()),
                    #[cfg(target_os = "linux")]
            handoff_coordinator: None,
            config,
        }
    }

    // ── Key rotation completion ──────────────────────────────────

    pub fn store_pending_rotation_confirm(
        &mut self,
        tx: tokio::sync::oneshot::Sender<Result<(), crate::v3::session::rotation::RotationError>>,
    ) {
        self.pending_rotation_confirm = Some(tx);
    }

    pub fn take_pending_rotation_confirm(
        &mut self,
    ) -> Option<tokio::sync::oneshot::Sender<Result<(), crate::v3::session::rotation::RotationError>>> {
        self.pending_rotation_confirm.take()
    }

    // ── Handoff coordinator ──────────────────────────────────────

    #[cfg(target_os = "linux")]
    pub fn set_handoff_coordinator(
        &mut self,
        coord: crate::v3::handoff::coordinator::HandoffCoordinator<
            crate::v3::handoff::transport::LinuxTransport,
            crate::v3::handoff::transport::LinuxMemfd,
        >,
    ) {
        self.handoff_coordinator = Some(coord);
    }

    #[cfg(target_os = "linux")]
    pub fn handoff_coordinator_mut(
        &mut self,
    ) -> Option<&mut crate::v3::handoff::coordinator::HandoffCoordinator<
        crate::v3::handoff::transport::LinuxTransport,
        crate::v3::handoff::transport::LinuxMemfd,
    >> {
        self.handoff_coordinator.as_mut()
    }

    // ── Early bulk chunk buffering ──────────────────────────────

    /// Buffer a bulk chunk that arrived before STREAM_OPEN created the
    /// reassembler. Replayed by `drain_early_chunks` when the reassembler
    /// is created. Bounded at 64 chunks per stream.
    pub fn buffer_early_chunk(&mut self, stream_id: u8, chunk_index: u32, data: PlaintextBuf, digest: [u8; 32]) {
        let chunks = self.early_bulk_chunks.entry(stream_id).or_default();
        if chunks.len() >= 64 {
            tracing::error!(stream_id, buffered = chunks.len(),
                "early_bulk_chunks: buffer full — dropping chunk (STREAM_OPEN never arrived?)");
            return;
        }
        tracing::debug!(stream_id, chunk_index, buffered = chunks.len() + 1,
            "early_bulk_chunks: buffering chunk (reassembler not yet created)");
        chunks.push((chunk_index, data, digest));
    }

    /// Drain all early chunks for a stream. Called after `create_reassembler`
    /// to replay chunks that arrived before STREAM_OPEN.
    pub fn drain_early_chunks(&mut self, stream_id: u8) -> Vec<(u32, PlaintextBuf, [u8; 32])> {
        self.early_bulk_chunks.remove(&stream_id).unwrap_or_default()
    }

    // ── Bulk data delivery staging ───────────────────────────────

    /// Push a delivered bulk chunk for the control loop to drain.
    /// Called by handlers after reassembler delivers in-order chunks.
    pub fn push_bulk_delivery(&mut self, stream_id: u8, chunk_index: u32, data: PlaintextBuf) {
        self.pending_bulk_deliveries.push((stream_id, chunk_index, data));
    }

    /// Drain all pending bulk deliveries. Called by the control loop
    /// after dispatch_frame returns.
    pub fn drain_bulk_deliveries(&mut self) -> Vec<(u8, u32, PlaintextBuf)> {
        std::mem::take(&mut self.pending_bulk_deliveries)
    }

    /// Stage a bulk transfer completion. Called by on_bulk_complete via the
    /// router. The control loop drains these AFTER pending_bulk_deliveries
    /// so the completion signal is ordered after all data chunks.
    pub fn push_bulk_completion(&mut self, stream_id: u8, transfer_id: uuid::Uuid, total_bytes: u64, total_chunks: u32) {
        self.pending_bulk_completions.push((stream_id, transfer_id, total_bytes, total_chunks));
    }

    pub fn drain_bulk_completions(&mut self) -> Vec<(u8, uuid::Uuid, u64, u32)> {
        std::mem::take(&mut self.pending_bulk_completions)
    }

    // ── Bulk FIN tracking (send side) ─────────────────────────────

    pub fn register_pending_fin(&mut self, stream_id: u8, state: PendingFinState) {
        self.pending_fins.insert(stream_id, state);
    }

    pub fn pending_fin_mut(&mut self, stream_id: u8) -> Option<&mut PendingFinState> {
        self.pending_fins.get_mut(&stream_id)
    }

    pub fn take_pending_fin(&mut self, stream_id: u8) -> Option<PendingFinState> {
        self.pending_fins.remove(&stream_id)
    }

    /// Check if a pending FIN's stream has all payload LinkInputs flushed
    /// through the outbound audit reorder buffer. Returns Some if ready.
    /// `reorder_next_expected` is the outbound reorder buffer's next_expected().
    pub fn take_ready_pending_fin(&mut self, stream_id: u8, reorder_next_expected: u64) -> Option<PendingFinState> {
        if let Some(state) = self.pending_fins.get(&stream_id) {
            if reorder_next_expected > state.last_chunk_seq {
                return self.pending_fins.remove(&stream_id);
            }
        }
        None
    }

    // ── Bulk FIN tracking (receive side) ──────────────────────────

    pub fn store_pending_fin_verify(&mut self, stream_id: u8, verify: PendingFinVerify) {
        self.pending_fin_verify.insert(stream_id, verify);
    }

    pub fn take_pending_fin_verify(&mut self, stream_id: u8) -> Option<PendingFinVerify> {
        self.pending_fin_verify.remove(&stream_id)
    }

    pub fn has_pending_fin_verify(&self, stream_id: u8) -> bool {
        self.pending_fin_verify.contains_key(&stream_id)
    }

    pub fn pending_fin_verify_ref(&self, stream_id: u8) -> Option<&PendingFinVerify> {
        self.pending_fin_verify.get(&stream_id)
    }

    pub fn pending_fins_stream_ids(&self) -> Vec<u8> {
        self.pending_fins.keys().copied().collect()
    }

    /// Iterator over pending requests for timeout sweep.
    pub(crate) fn pending_requests_iter(&self) -> impl Iterator<Item = (&uuid::Uuid, &PendingRequest)> {
        self.pending_requests.iter()
    }
}
