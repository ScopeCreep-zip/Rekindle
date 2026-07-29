//! Test helpers for dispatch and handler tests.
//!
//! SSOT for test state construction and bulk delivery assertions.
//! When delivery internals change, these helpers change — not 40 test files.
//!
//! Under the lane-sharded architecture, tests construct per-lane state
//! structs (DataState, ControlState, etc.) and call handlers with
//! explicit parameters — the same way dispatch/inbound.rs does.

use std::collections::HashMap;
use std::sync::Arc;

use crate::v4::audit::retention::RetentionBuffer;
use crate::v4::codec::envelope::EnvelopeInfo;
use crate::v4::codec::header::StreamHeaderInfo;
use crate::v4::config::SessionConfig;
use crate::v4::crypto::keys::derive_all_keys;
use crate::v4::dedup::cache::{ReceiverCache, SenderCache};
use crate::v4::handlers::channel::pending_requests::PendingRequestTracker;
use crate::v4::handlers::channel::subscribe::SubscriptionRegistry;
use crate::v4::io::control_loop::lane::state::{
    AuditState, ControlState, DataState, HandoffState,
};
use crate::v4::io::control_loop::shared_state::{SharedAuditLinks, SharedSessionState, CipherKeySet, SessionStateHandle};
use crate::v4::io::encode::EpochKeys;
use crate::v4::io::epoch_signal::EpochSignal;
use crate::v4::io::lane_channels::PlaintextBuf;
use crate::v4::router::{ConnectionInfo, MockRouter};
use crate::v4::session::handshake::{
    HandshakeResult, AEAD_CODE_AES256GCM, build_encoder_decoder,
};
use crate::v4::session::rotation::RotationCoordinator;
use crate::v4::session::SessionRole;
use crate::v4::stream::flow_control::BackpressureState;
use crate::v4::stream::registry::StreamRegistry;
use crate::v4::stream::resume::ResumeRegistry;
use crate::v4::wire::capability::CapabilityBits;
use crate::v4::wire::clearance::Clearance;
use crate::v4::wire::constants::WIRE_VERSION;
use crate::v4::wire::frame_class::FrameClass;
use crate::v4::wire::frame_kind::{
    AuditKind, ChannelKind, DatagramKind, HandoffKind, StreamKind,
};
use crate::v4::wire::lane::Lane;

// ── Handshake result factory ────────────────────────────────────

pub fn default_handshake_result(clearance: Clearance) -> HandshakeResult {
    let keys = derive_all_keys(&[0xCC; 32]);
    let (encoder, decoder) = build_encoder_decoder(&keys, AEAD_CODE_AES256GCM, true)
        .expect("AES-256-GCM init must succeed");
    HandshakeResult {
        session_id: uuid::Uuid::from_u128(1),
        local_peer_id: [0xAA; 32],
        remote_peer_id: [0xBB; 32],
        agreed_clearance: clearance,
        active_capabilities: CapabilityBits::MANDATORY_V1
            | CapabilityBits::SUBSCRIPTION
            | CapabilityBits::RESUME
            | CapabilityBits::DEDUP_CACHE
            | CapabilityBits::SHARED_ARENA,
        agreed_aead: AEAD_CODE_AES256GCM,
        handshake_hash: [0xCC; 32],
        keys,
        encoder,
        decoder,
    }
}

// ── Per-lane state factories ────────────────────────────────────

pub fn make_test_data_state() -> DataState {
    make_test_data_state_with_config(SessionConfig::default())
}

pub fn make_test_data_state_with_config(config: SessionConfig) -> DataState {
    DataState {
        stream_registry: StreamRegistry::new(),
        reassemblers: HashMap::new(),
        stream_credits: HashMap::new(),
        lane_credit_bytes: HashMap::new(),
        backpressure: BackpressureState::new(),
        chunk_to_seq: HashMap::new(),
        resume_registry: ResumeRegistry::new(),
        sender_cache: SenderCache::new(config.sender_cache_config.clone()),
        receiver_cache: ReceiverCache::new(config.receiver_cache_config.clone()),
        pending_fin_verify: HashMap::new(),
        early_bulk_chunks: HashMap::new(),
        pending_bulk_deliveries: Vec::new(),
        pending_bulk_completions: Vec::new(),
        pending_fins: HashMap::new(),
        pending_outbound: Vec::new(),
        retention: Arc::new(parking_lot::Mutex::new(
            RetentionBuffer::new(config.retention_config.clone()),
        )),
        agreed_clearance: Clearance::Internal,
        active_capabilities: CapabilityBits::MANDATORY_V1,
        config: Arc::new(config),
    }
}

pub fn make_test_control_state() -> (ControlState, tokio::sync::mpsc::Receiver<crate::v4::io::control_loop::lane::data::DataRevocation>, tokio::sync::mpsc::Receiver<crate::v4::io::control_loop::audit_merge::AuditLinkDirection>) {
    let config = SessionConfig::default();
    let keys = derive_all_keys(&[0xCC; 32]);
    let (revocation_tx, revocation_rx) = tokio::sync::mpsc::channel(64);
    let (audit_merge_tx, audit_merge_rx) = tokio::sync::mpsc::channel(64);
    let state = ControlState {
        rotation: RotationCoordinator::new(keys.clone(), 0),
        pending_requests: PendingRequestTracker::new(config.max_pending_requests),
        subscriptions: SubscriptionRegistry::new(config.max_subscriptions),
        send_seq: 0,
        last_ping_nonce: None, previous_ping_nonce: None,
        last_pong_received: None, heartbeat_miss_count: 0,
        remote_last_seen_our_seq: 0, peer_final_session_seq: None,
        local_goodbye_sent: false,
        quiescence_deadline: None, rotation_deadline: None, drain_deadline: None,
        pending_rotation_confirm: None,
        keys, role: SessionRole::Dialler, agreed_aead: AEAD_CODE_AES256GCM,
        agreed_clearance: Clearance::Internal,
        active_capabilities: CapabilityBits::MANDATORY_V1,
        signal_epoch: 0, epoch_signal: Arc::new(EpochSignal::new()),
        audit_links: Arc::new(SharedAuditLinks::new([0xCC; 32])),
        revocation_tx,
        audit_merge_tx,
        config: Arc::new(config),
    };
    (state, revocation_rx, audit_merge_rx)
}

pub fn make_test_shared_state() -> SessionStateHandle {
    let keys = derive_all_keys(&[0xCC; 32]);
    let initial_cipher = CipherKeySet {
        epoch: 0,
        keys: EpochKeys {
            envelope_key: keys.envelope_d2l,
            header_key: keys.header_d2l,
            cipher: crate::v4::codec::aead::FrameCipher::aes256gcm(
                &keys.stream_d2l,
                crate::v4::wire::constants::DIRECTION_ID_D2L,
            ).expect("cipher init"),
        },
    };
    Arc::new(parking_lot::RwLock::new(SharedSessionState {
        cipher_keys: Arc::new(initial_cipher),
        shutting_down: false,
        session_state: crate::v4::session::state::SessionState::Established,
        capabilities: CapabilityBits::MANDATORY_V1,
    }))
}

pub fn make_test_connection_info() -> ConnectionInfo {
    ConnectionInfo {
        conn_id: 1,
        session_id: uuid::Uuid::from_u128(1),
        peer_id: [0xBB; 32],
        clearance: Clearance::Internal,
        capabilities: CapabilityBits::MANDATORY_V1,
    }
}

pub fn make_test_audit_state() -> (AuditState, tokio::sync::mpsc::Receiver<crate::v4::io::control_loop::audit_merge::AuditLinkDirection>) {
    let config = SessionConfig::default();
    let (audit_merge_tx, audit_merge_rx) = tokio::sync::mpsc::channel(64);
    let retention = Arc::new(parking_lot::Mutex::new(
        RetentionBuffer::new(config.retention_config.clone()),
    ));
    let state = AuditState {
        retention,
        verified_proofs: HashMap::new(),
        audit_links: Arc::new(SharedAuditLinks::new([0xCC; 32])),
        audit_merge_tx,
    };
    (state, audit_merge_rx)
}

pub fn make_test_handoff_state() -> HandoffState {
    HandoffState {
        #[cfg(target_os = "linux")]
        arenas: Vec::new(),
        #[cfg(target_os = "linux")]
        sidechannel: None,
        #[cfg(target_os = "linux")]
        pending_arena_fds: None,
        conn_id: 1,
    }
}

// ── Reassembler test helper ─────────────────────────────────────

/// Insert a chunk into a DataState's reassembler and stage delivered chunks.
pub fn insert_chunk(state: &mut DataState, stream_id: u8, chunk_index: u32, data: &[u8]) {
    let digest = *blake3::hash(data).as_bytes();
    let delivered = state.reassemblers.get_mut(&stream_id)
        .expect("reassembler must exist for stream_id")
        .insert_with_digest(chunk_index, PlaintextBuf::Owned(data.to_vec()), digest);
    for (ci, d) in delivered {
        state.pending_bulk_deliveries.push((stream_id, ci, d));
    }
}

// ── Bulk delivery assertions ────────────────────────────────────

pub fn assert_bulk_delivered(state: &mut DataState, stream_id: u8, expected: &[&[u8]]) {
    let deliveries = std::mem::take(&mut state.pending_bulk_deliveries);
    let for_stream: Vec<_> = deliveries.into_iter()
        .filter(|(sid, _, _)| *sid == stream_id)
        .collect();
    assert_eq!(
        for_stream.len(), expected.len(),
        "expected {} bulk deliveries for stream {stream_id}, got {}",
        expected.len(), for_stream.len(),
    );
    for (i, (_, _, data)) in for_stream.iter().enumerate() {
        assert_eq!(
            &**data, expected[i],
            "bulk delivery {i} for stream {stream_id} data mismatch"
        );
    }
}

pub fn assert_no_bulk_deliveries(state: &mut DataState) {
    assert!(
        state.pending_bulk_deliveries.is_empty(),
        "expected zero bulk deliveries, got {}",
        state.pending_bulk_deliveries.len(),
    );
}

pub fn assert_no_router_deliveries(router: &MockRouter) {
    assert_eq!(router.requests.lock().len(), 0, "unexpected request");
    assert_eq!(router.notifications.lock().len(), 0, "unexpected notification");
    assert_eq!(router.publishes.lock().len(), 0, "unexpected publish");
    assert_eq!(router.replies.lock().len(), 0, "unexpected reply");
    assert_eq!(router.rejects.lock().len(), 0, "unexpected reject");
    assert_eq!(router.bulk_completes.lock().len(), 0, "unexpected bulk complete");
    assert_eq!(router.bulk_failures.lock().len(), 0, "unexpected bulk failure");
    assert_eq!(router.bulk_chunks.lock().len(), 0, "unexpected bulk chunk");
    assert_eq!(router.route_frames.lock().len(), 0, "unexpected generic frame");
}

// ── Wire format test helpers ────────────────────────────────────

pub fn test_envelope(lane: Lane) -> EnvelopeInfo {
    EnvelopeInfo {
        wire_version: WIRE_VERSION,
        lane, flags: 0, body_len: 100, session_seq: 1,
    }
}

pub fn test_stream_header(kind: StreamKind) -> StreamHeaderInfo {
    StreamHeaderInfo {
        frame_class: FrameClass::Stream,
        frame_kind: kind,
        stream_id: 0, header_flags: 0, chunk_index: 0, nonce: 0,
    }
}

pub fn make_minimal_channel_payload(kind: ChannelKind) -> Vec<u8> {
    vec![FrameClass::Channel as u8, kind as u8, 0, 0]
}

pub fn make_minimal_stream_payload(kind: StreamKind) -> Vec<u8> {
    vec![FrameClass::Stream as u8, kind as u8, 0, 0]
}

pub fn make_minimal_datagram_payload(kind: DatagramKind) -> Vec<u8> {
    vec![FrameClass::Datagram as u8, kind as u8, 0, 0]
}

pub fn make_minimal_audit_payload(kind: AuditKind) -> Vec<u8> {
    vec![FrameClass::Audit as u8, kind as u8, 0, 0]
}

pub fn make_minimal_handoff_payload(kind: HandoffKind) -> Vec<u8> {
    vec![FrameClass::Handoff as u8, kind as u8, 0, 0]
}
