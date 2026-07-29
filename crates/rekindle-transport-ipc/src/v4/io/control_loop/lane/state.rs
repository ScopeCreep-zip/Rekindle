//! Per-lane state structs — the interface contract.
//!
//! Each struct groups exactly the fields its lane's handlers mutate.
//! dispatch/inbound.rs destructures these and passes references to
//! each handler. A handler cannot access state outside its parameter
//! list — the compiler enforces isolation.
//!
//! Shared concerns live in their owning tasks:
//! - `session_state` → `SharedSessionState` (RwLock, Control lane writes)
//! - `recv_last_seq` → `SharedAuditLinks` (audit_merge publishes)
//! - Checkpoint cadence → audit_merge (both directions)
//! - Gap detection → audit_merge (inbound reorder stall)

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use tokio::sync::{mpsc, oneshot};

use parking_lot::Mutex;

use crate::v4::audit::retention::RetentionBuffer;
use crate::v4::audit::VerifiedProof;
use crate::v4::config::SessionConfig;
use crate::v4::crypto::keys::DerivedKeys;
use crate::v4::dedup::cache::{ReceiverCache, SenderCache};
use crate::v4::handlers::channel::pending_requests::PendingRequestTracker;
use crate::v4::handlers::channel::subscribe::SubscriptionRegistry;
use crate::v4::io::control_loop::audit_merge::AuditLinkDirection;
use crate::v4::io::control_loop::fin::{PendingFinState, PendingFinVerify};
use crate::v4::io::control_loop::lane::data::DataRevocation;
use crate::v4::io::control_loop::shared_state::SharedAuditLinks;
use crate::v4::io::epoch_signal::EpochSignal;
use crate::v4::io::lane_channels::PlaintextBuf;
use crate::v4::session::rotation::{RotationCoordinator, RotationError};
use crate::v4::session::SessionRole;
use crate::v4::stream::flow_control::{BackpressureState, CreditTracker};
use crate::v4::stream::reassembler::Reassembler;
use crate::v4::stream::registry::StreamRegistry;
use crate::v4::stream::resume::ResumeRegistry;
use crate::v4::wire::capability::CapabilityBits;
use crate::v4::wire::clearance::Clearance;
use crate::v4::wire::outbound::OutboundFrame;

#[cfg(target_os = "linux")]
use crate::v4::streaming::shared_arena::SharedArena;
#[cfg(target_os = "linux")]
use crate::v4::streaming::sidechannel::SideChannel;

/// Handoff lane: Streaming (0x05) frames.
pub struct HandoffState {
    #[cfg(target_os = "linux")]
    pub arenas: Vec<Arc<SharedArena>>,
    #[cfg(target_os = "linux")]
    pub sidechannel: Option<Arc<SideChannel>>,
    #[cfg(target_os = "linux")]
    pub pending_arena_fds: Option<(std::os::unix::io::OwnedFd, std::os::unix::io::OwnedFd)>,
    pub conn_id: u64,
}

/// Data lane: Stream (0x02) frames + rerouted Credit/Backpressure.
pub struct DataState {
    pub stream_registry: StreamRegistry,
    pub reassemblers: HashMap<u8, Reassembler>,
    pub stream_credits: HashMap<u8, CreditTracker>,
    pub lane_credit_bytes: HashMap<u8, u32>,
    pub backpressure: BackpressureState,
    pub chunk_to_seq: HashMap<(u8, u32), u64>,
    pub resume_registry: ResumeRegistry,
    pub sender_cache: SenderCache,
    pub receiver_cache: ReceiverCache,
    pub pending_fin_verify: HashMap<u8, PendingFinVerify>,
    pub early_bulk_chunks: HashMap<u8, Vec<(u32, PlaintextBuf, [u8; 32])>>,
    pub pending_bulk_deliveries: Vec<(u8, u32, PlaintextBuf)>,
    pub pending_bulk_completions: Vec<(u8, uuid::Uuid, u64, u32)>,
    pub pending_fins: HashMap<u8, PendingFinState>,
    pub pending_outbound: Vec<OutboundFrame>,
    /// Shared with AuditState via `Arc<Mutex>`.
    pub retention: Arc<Mutex<RetentionBuffer>>,
    pub agreed_clearance: Clearance,
    pub active_capabilities: CapabilityBits,
    pub config: Arc<SessionConfig>,
}

/// Control lane: Channel (0x01) + Datagram (0x03) frames.
pub struct ControlState {
    pub rotation: RotationCoordinator,
    pub pending_requests: PendingRequestTracker,
    pub subscriptions: SubscriptionRegistry,
    pub send_seq: u64,
    pub last_ping_nonce: Option<u64>,
    pub previous_ping_nonce: Option<u64>,
    pub last_pong_received: Option<Instant>,
    pub heartbeat_miss_count: u32,
    pub remote_last_seen_our_seq: u64,
    pub peer_final_session_seq: Option<u64>,
    pub local_goodbye_sent: bool,
    pub quiescence_deadline: Option<Instant>,
    pub rotation_deadline: Option<Instant>,
    pub drain_deadline: Option<Instant>,
    pub pending_rotation_confirm: Option<oneshot::Sender<Result<(), RotationError>>>,
    pub keys: DerivedKeys,
    pub role: SessionRole,
    pub agreed_aead: u8,
    pub agreed_clearance: Clearance,
    pub active_capabilities: CapabilityBits,
    pub signal_epoch: u8,
    pub epoch_signal: Arc<EpochSignal>,
    pub audit_links: Arc<SharedAuditLinks>,
    pub revocation_tx: mpsc::Sender<DataRevocation>,
    pub audit_merge_tx: mpsc::Sender<AuditLinkDirection>,
    pub config: Arc<SessionConfig>,
}

/// Audit lane: Audit (0x04) frames.
pub struct AuditState {
    /// Shared with DataState via `Arc<Mutex>`.
    pub retention: Arc<Mutex<RetentionBuffer>>,
    pub verified_proofs: HashMap<uuid::Uuid, VerifiedProof>,
    pub audit_links: Arc<SharedAuditLinks>,
    pub audit_merge_tx: mpsc::Sender<AuditLinkDirection>,
}
