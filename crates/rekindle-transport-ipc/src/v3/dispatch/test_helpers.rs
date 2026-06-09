//! Test helpers for dispatch and handler tests.
//!
//! SSOT for test context construction and bulk delivery assertions.
//! When delivery internals change, these helpers change — not 40 test files.

use std::sync::Arc;

use crate::v3::io::lane_channels::PlaintextBuf;
use crate::v3::codec::envelope::EnvelopeInfo;
use crate::v3::codec::header::StreamHeaderInfo;
use crate::v3::context::{SessionConfig, SessionContext, SessionRole};
use crate::v3::crypto::keys::derive_all_keys;
use crate::v3::router::MockRouter;
use crate::v3::session::handshake::{
    HandshakeResult, AEAD_CODE_AES256GCM, build_encoder_decoder,
};
use crate::v3::wire::capability::CapabilityBits;
use crate::v3::wire::clearance::Clearance;
use crate::v3::wire::constants::WIRE_VERSION;
use crate::v3::wire::frame_class::FrameClass;
use crate::v3::wire::frame_kind::{
    ChannelKind, StreamKind, DatagramKind, AuditKind, HandoffKind,
};
use crate::v3::wire::lane::Lane;

// ── Single factory for all test contexts ─────────────────────────

fn default_handshake_result(clearance: Clearance) -> HandshakeResult {
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
            | CapabilityBits::HANDOFF_MEMFD,
        agreed_aead: AEAD_CODE_AES256GCM,
        handshake_hash: [0xCC; 32],
        keys,
        encoder,
        decoder,
    }
}

fn build_context(
    hr: HandshakeResult,
    config: SessionConfig,
    router: Arc<MockRouter>,
) -> SessionContext {
    let aead = hr.agreed_aead;
    SessionContext::from_handshake(hr, config, router, 1, SessionRole::Dialler, aead)
}

/// Build a SessionContext with default config and Internal clearance.
pub fn make_test_context() -> (SessionContext, Arc<MockRouter>) {
    make_test_context_with_config(SessionConfig::default())
}

/// Build a SessionContext with custom config.
pub fn make_test_context_with_config(config: SessionConfig) -> (SessionContext, Arc<MockRouter>) {
    let router = MockRouter::new();
    let hr = default_handshake_result(Clearance::Internal);
    let ctx = build_context(hr, config, router.clone());
    (ctx, router)
}

/// Build a SessionContext with a specific clearance level.
pub fn make_test_context_with_clearance(clearance: Clearance) -> (SessionContext, Arc<MockRouter>) {
    let router = MockRouter::new();
    let hr = default_handshake_result(clearance);
    let ctx = build_context(hr, SessionConfig::default(), router.clone());
    (ctx, router)
}

// ── Reassembler test helper ──────────────────────────────────────

/// Insert a chunk into a stream's reassembler and stage delivered chunks.
/// SSOT for the hash + insert + push_bulk_delivery pattern used by every
/// reassembler test. When the reassembler API changes, this changes — not
/// every test file.
pub fn insert_chunk(ctx: &mut SessionContext, stream_id: u8, chunk_index: u32, data: &[u8]) {
    let digest = *blake3::hash(data).as_bytes();
    let delivered = ctx.reassembler_mut(stream_id)
        .expect("reassembler must exist for stream_id")
        .insert_with_digest(chunk_index, PlaintextBuf::Owned(data.to_vec()), digest);
    for (ci, d) in delivered {
        ctx.push_bulk_delivery(stream_id, ci, d);
    }
}

// ── Bulk delivery assertions ─────────────────────────────────────

/// Assert that the handler delivered exactly the expected chunks for
/// the given stream_id. Drains pending_bulk_deliveries.
pub fn assert_bulk_delivered(ctx: &mut SessionContext, stream_id: u8, expected: &[&[u8]]) {
    let deliveries = ctx.drain_bulk_deliveries();
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

/// Assert that no bulk deliveries are pending.
pub fn assert_no_bulk_deliveries(ctx: &mut SessionContext) {
    let deliveries = ctx.drain_bulk_deliveries();
    assert!(
        deliveries.is_empty(),
        "expected zero bulk deliveries, got {}",
        deliveries.len(),
    );
}

/// Assert that the MockRouter received zero application-level deliveries.
pub fn assert_no_router_deliveries(router: &MockRouter) {
    assert_eq!(router.requests.lock().len(), 0, "unexpected request delivery to router");
    assert_eq!(router.notifications.lock().len(), 0, "unexpected notification delivery to router");
    assert_eq!(router.publishes.lock().len(), 0, "unexpected publish delivery to router");
    assert_eq!(router.replies.lock().len(), 0, "unexpected reply delivery to router");
    assert_eq!(router.rejects.lock().len(), 0, "unexpected reject delivery to router");
    assert_eq!(router.bulk_completes.lock().len(), 0, "unexpected bulk complete delivery to router");
    assert_eq!(router.bulk_failures.lock().len(), 0, "unexpected bulk failure delivery to router");
    assert_eq!(router.route_frames.lock().len(), 0, "unexpected generic frame delivery to router");
}

pub fn test_envelope(lane: Lane) -> EnvelopeInfo {
    EnvelopeInfo {
        wire_version: WIRE_VERSION,
        lane,
        flags: 0,
        body_len: 100,
        session_seq: 1,
    }
}

pub fn test_stream_header(kind: StreamKind) -> StreamHeaderInfo {
    StreamHeaderInfo {
        frame_class: FrameClass::Stream,
        frame_kind: kind,
        stream_id: 0,
        header_flags: 0,
        chunk_index: 0,
        nonce: 0,
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
