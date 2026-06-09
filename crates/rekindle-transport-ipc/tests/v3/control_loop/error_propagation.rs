//! Error propagation tests.
//!
//! The control loop must propagate every error as a named, specific
//! outcome — never 0xFFFF, never silent, never swallowed. These tests
//! verify the error mapping from handlers to wire failure codes and
//! the resolution of pending operations on session termination.
//!
//! Application developers depend on specific failure codes to show
//! meaningful error messages ("transfer failed: content hash mismatch"
//! not "unknown error 0xFFFF"). SREs depend on structured tracing
//! with failure codes for alerting and dashboards.

use rekindle_transport_ipc::v3::context::OutboundFrame;
use rekindle_transport_ipc::v3::dispatch::test_helpers::make_test_context;
use rekindle_transport_ipc::v3::handlers::HandlerError;
use rekindle_transport_ipc::v3::session::state::SessionState;
use rekindle_transport_ipc::v3::wire::failure::FailureCode;

/// Every HandlerError variant maps to a specific FailureCode.
/// No variant maps to 0xFFFF. This is the contract between
/// handlers and the wire protocol.
#[test]
fn every_handler_error_has_specific_failure_code() {
    let errors_and_expected: Vec<(HandlerError, FailureCode)> = vec![
        (HandlerError::FrameDisallowedInState, FailureCode::FrameDisallowedInState),
        (HandlerError::ClearanceInsufficient, FailureCode::ClearanceInsufficient),
        (HandlerError::StreamAlreadyOpen(0), FailureCode::StreamAlreadyOpen),
        (HandlerError::StreamIdExhausted, FailureCode::StreamIdExhausted),
        (HandlerError::StreamNotFound(0), FailureCode::FrameDisallowedInState),
        (HandlerError::TransferIdUnknown(uuid::Uuid::nil()), FailureCode::TransferIdUnknown),
        (HandlerError::ResumeWindowExpired, FailureCode::ResumeWindowExpired),
        (HandlerError::AuditAnchorMismatch, FailureCode::AuditAnchorMismatch),
        (HandlerError::ContentAnchorMismatch, FailureCode::ContentAnchorMismatch),
        (HandlerError::ContentHashMismatch, FailureCode::ContentHashMismatch),
        (HandlerError::DedupCacheMiss, FailureCode::DedupCacheMiss),
        (HandlerError::DedupClearanceInsufficient, FailureCode::DedupClearanceInsufficient),
        (HandlerError::PendingRequestsExhausted, FailureCode::PendingRequestsExhausted),
        (HandlerError::SubscriptionQuotaExhausted, FailureCode::SubscriptionQuotaExhausted),
        (HandlerError::BackpressureExhausted, FailureCode::BackpressureExhausted),
        (HandlerError::CreditExhausted, FailureCode::BudgetExhausted),
        (HandlerError::NonceMismatch, FailureCode::ReplayDetected),
        (HandlerError::NoPendingPing, FailureCode::HeartbeatTimeout),
        (HandlerError::CodecFailed("test".into()), FailureCode::FrameMalformed),
    ];

    for (error, expected_code) in errors_and_expected {
        let actual = error.failure_code();
        assert_eq!(
            actual, expected_code,
            "HandlerError::{:?} must map to {:?}, got {:?}",
            error, expected_code, actual
        );
    }
}

/// failure_code() never returns a value outside the FailureCode enum.
/// The wire protocol carries this as a u32 — invalid values cause
/// the peer to fail with InvalidFailureCode.
#[test]
fn failure_code_is_valid_wire_value() {
    let errors: Vec<HandlerError> = vec![
        HandlerError::FrameDisallowedInState,
        HandlerError::ClearanceInsufficient,
        HandlerError::ContentHashMismatch,
        HandlerError::CodecFailed("x".into()),
    ];

    for error in errors {
        let code = error.failure_code();
        let wire_val = code as u32;
        // Every FailureCode has a non-zero wire value in a known category
        assert_ne!(wire_val, 0, "failure code must not be zero");
        assert_ne!(wire_val, 0xFFFF, "failure code must not be 0xFFFF placeholder");
        let category = (wire_val >> 16) as u16;
        assert!(
            (1..=7).contains(&category),
            "failure code category must be 1-7, got {category} for {code:?}"
        );
    }
}

/// CHANNEL_ERROR handler transitions to Closed.
/// When a peer sends us a CHANNEL_ERROR, the session is over.
/// The application receives on_connection_state_change.
#[test]
fn channel_error_transitions_to_closed() {
    let (mut ctx, _router) = make_test_context();

    use rekindle_transport_ipc::v3::handlers::channel::error;
    let payload = {
        let mut buf = Vec::new();
        buf.extend_from_slice(&0x0001u16.to_le_bytes()); // error_code
        buf.extend_from_slice(&[0, 0]); // reserved
        buf.extend_from_slice(&0u32.to_le_bytes()); // message_len=0
        buf
    };
    error::handle(&mut ctx, &payload).unwrap();
    assert_eq!(ctx.session_state(), SessionState::Closed);
}

/// push_channel_error produces a CHANNEL_ERROR outbound frame.
/// Handlers call this to surface protocol errors to the peer.
#[test]
fn push_channel_error_produces_outbound_frame() {
    let (mut ctx, _router) = make_test_context();

    ctx.push_channel_error(0x0003, "test error message");

    let out = ctx.drain_outbound();
    assert_eq!(out.len(), 1);
    match &out[0] {
        OutboundFrame::Channel { kind: rekindle_transport_ipc::v3::wire::frame_kind::ChannelKind::ChannelError, payload } => {
            let error_code = u16::from_le_bytes([payload[0], payload[1]]);
            assert_eq!(error_code, 0x0003);
        }
        other => panic!("expected CHANNEL_ERROR, got {other:?}"),
    }
}

/// Pending request sweep removes expired requests.
/// The control loop calls sweep_expired_requests on every heartbeat
/// tick to prevent unbounded HashMap growth.
#[test]
fn sweep_expired_requests_removes_stale() {
    let (mut ctx, _router) = make_test_context();

    // Register a request with 0ms timeout — immediately expired
    ctx.register_pending_request(uuid::Uuid::from_u128(1), 0);
    ctx.register_pending_request(uuid::Uuid::from_u128(2), 0);
    ctx.register_pending_request(uuid::Uuid::from_u128(3), 1_000_000); // far future

    assert_eq!(ctx.pending_request_count(), 3);

    // Small delay so the 0ms requests are actually expired
    std::thread::sleep(std::time::Duration::from_millis(1));

    let expired = ctx.sweep_expired_requests();
    assert_eq!(expired.len(), 2, "two 0ms-timeout requests must be swept");
    assert_eq!(
        ctx.pending_request_count(),
        1,
        "one non-expired request must remain"
    );
}

/// Pending request sweep with no expired requests is a no-op.
#[test]
fn sweep_with_no_expired_is_noop() {
    let (mut ctx, _router) = make_test_context();

    ctx.register_pending_request(uuid::Uuid::from_u128(1), 60_000);
    ctx.register_pending_request(uuid::Uuid::from_u128(2), 60_000);

    let expired = ctx.sweep_expired_requests();
    assert!(expired.is_empty());
    assert_eq!(ctx.pending_request_count(), 2);
}

/// RTT estimate is available after PONG received.
/// Applications use RTT for ack window calibration and
/// resume eligibility window computation.
#[test]
fn rtt_estimate_available_after_pong() {
    let (mut ctx, _router) = make_test_context();

    assert!(ctx.rtt_estimate().is_none(), "no RTT before first PONG");

    ctx.record_pong_received();

    let rtt = ctx.rtt_estimate();
    assert!(rtt.is_some(), "RTT must be available after PONG");
    // The RTT is Instant::now() - last_pong_received, which is
    // microseconds since we just set it. It should be very small.
    assert!(
        rtt.unwrap() < std::time::Duration::from_secs(1),
        "RTT must be small — we just recorded the PONG"
    );
}
