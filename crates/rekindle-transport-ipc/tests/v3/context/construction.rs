use rekindle_transport_ipc::v3::context::{SessionContext, SessionConfig, SessionRole};
use rekindle_transport_ipc::v3::crypto::keys::derive_all_keys;
use rekindle_transport_ipc::v3::router::MockRouter;
use rekindle_transport_ipc::v3::session::handshake::{
    HandshakeResult, AEAD_CODE_AES256GCM, build_encoder_decoder,
};
use rekindle_transport_ipc::v3::wire::capability::CapabilityBits;
use rekindle_transport_ipc::v3::wire::clearance::Clearance;
use rekindle_transport_ipc::v3::session::state::SessionState;

const TEST_HASH: [u8; 32] = [0xCC; 32];

fn test_handshake_result() -> HandshakeResult {
    let keys = derive_all_keys(&TEST_HASH);
    let (encoder, decoder) = build_encoder_decoder(&keys, AEAD_CODE_AES256GCM, true)
        .expect("AES-256-GCM init");
    HandshakeResult {
        session_id: uuid::Uuid::from_u128(1),
        local_peer_id: [0xAA; 32],
        remote_peer_id: [0xBB; 32],
        agreed_clearance: Clearance::Internal,
        active_capabilities: CapabilityBits::MANDATORY_V1,
        agreed_aead: AEAD_CODE_AES256GCM,
        handshake_hash: TEST_HASH,
        keys,
        encoder,
        decoder,
    }
}

fn build_test_context(hr: HandshakeResult) -> SessionContext {
    let router = MockRouter::new();
    let aead = hr.agreed_aead;
    SessionContext::from_handshake(hr, SessionConfig::default(), router, 1, SessionRole::Dialler, aead)
}

#[test]
fn from_handshake_populates_session_id() {
    let ctx = build_test_context(test_handshake_result());
    assert_eq!(ctx.session_id(), uuid::Uuid::from_u128(1));
}

#[test]
fn from_handshake_populates_peer_ids() {
    let ctx = build_test_context(test_handshake_result());
    assert_eq!(ctx.local_peer_id(), &[0xAA; 32]);
    assert_eq!(ctx.remote_peer_id(), &[0xBB; 32]);
}

#[test]
fn from_handshake_populates_clearance() {
    let ctx = build_test_context(test_handshake_result());
    assert_eq!(ctx.agreed_clearance(), Clearance::Internal);
}

#[test]
fn from_handshake_populates_capabilities() {
    let ctx = build_test_context(test_handshake_result());
    assert_eq!(ctx.active_capabilities(), CapabilityBits::MANDATORY_V1);
}

#[test]
fn initial_state_is_established() {
    let ctx = build_test_context(test_handshake_result());
    assert_eq!(ctx.session_state(), SessionState::Established);
}

#[test]
fn initial_send_seq_is_zero() {
    let ctx = build_test_context(test_handshake_result());
    assert_eq!(ctx.send_seq(), 0);
}

#[test]
fn initial_recv_last_seq_is_zero() {
    let ctx = build_test_context(test_handshake_result());
    assert_eq!(ctx.recv_last_seq(), 0);
}

#[test]
fn audit_chains_anchored_to_handshake_hash() {
    let ctx = build_test_context(test_handshake_result());
    assert_eq!(ctx.outbound_chain().anchor_record().value, TEST_HASH);
    assert_eq!(ctx.inbound_chain().anchor_record().value, TEST_HASH);
}

#[test]
fn stream_registry_empty_initially() {
    let ctx = build_test_context(test_handshake_result());
    assert_eq!(ctx.stream_registry().active_count(), 0);
}

#[test]
fn no_pending_requests_initially() {
    let ctx = build_test_context(test_handshake_result());
    assert_eq!(ctx.pending_request_count(), 0);
}

#[test]
fn no_subscriptions_initially() {
    let ctx = build_test_context(test_handshake_result());
    assert_eq!(ctx.subscription_count(), 0);
}

#[test]
fn outbound_queue_empty_initially() {
    let mut ctx = build_test_context(test_handshake_result());
    assert!(ctx.drain_outbound().is_empty());
}

#[test]
fn keys_match_handshake() {
    let expected_keys = derive_all_keys(&TEST_HASH);
    let ctx = build_test_context(test_handshake_result());
    assert_eq!(ctx.keys().envelope_d2l, expected_keys.envelope_d2l);
    assert_eq!(ctx.keys().stream_d2l, expected_keys.stream_d2l);
    assert_eq!(ctx.keys().audit_d2l, expected_keys.audit_d2l);
    assert_eq!(ctx.keys().handoff, expected_keys.handoff);
}
