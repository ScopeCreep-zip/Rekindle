use rekindle_transport_ipc::v3::codec::channel::rotate as rotate_codec;
use rekindle_transport_ipc::v3::context::OutboundFrame;
use rekindle_transport_ipc::v3::dispatch::test_helpers::{make_test_context, assert_no_router_deliveries};
use rekindle_transport_ipc::v3::handlers::channel::rotate;
use rekindle_transport_ipc::v3::session::state::SessionState;
use rekindle_transport_ipc::v3::wire::frame_kind::{AuditKind, ChannelKind};

/// handle_init (responder path): receives ROTATE_INIT, produces ROTATE_COMMIT,
/// queues both decoder and encoder keys via EpochSignal, and transitions
/// Established → Rotating → Established (via RotateCommitSent).
///
/// The responder completes its rotation handshake in a single handler call.
/// The session returns to Established immediately — the read task installs
/// the new keys atomically before reading the next frame.
#[test]
fn rotate_init_produces_commit_and_returns_to_established() {
    let (mut responder, router) = make_test_context();
    let init = rotate_codec::RotateInitPayload {
        rotation_id: uuid::Uuid::from_u128(42),
        initiator_generation: 1,
        phase_2_deadline_ms: 30_000,
        new_static_pub: [0xDD; 32],
        rotation_chain_secret: [0xEE; 32],
        transcript_anchor: [0xFF; 32],
    };
    let encoded = rotate_codec::encode_init(&init);
    rotate::handle_init(&mut responder, &encoded).unwrap();

    // Responder transitions through Rotating and back to Established
    // in a single handler call (RotateInitReceived → RotateCommitSent).
    assert_eq!(responder.session_state(), SessionState::Established);

    let out = responder.drain_outbound();
    assert!(
        out.iter().any(|f| matches!(f, OutboundFrame::Channel { kind: ChannelKind::RotateCommit, .. })),
        "rotate_init must produce ROTATE_COMMIT"
    );

    // Epoch signal must have a pending install with both decoder and encoder keys.
    // The read task drains this signal and installs the keys atomically.
    let epoch_install = responder.epoch_signal().drain();
    assert!(
        epoch_install.is_some(),
        "responder must have queued epoch keys via install_next_epoch_keys after handle_init"
    );

    assert_no_router_deliveries(&router);
}

/// handle_commit (initiator path): receives ROTATE_COMMIT, derives new keys,
/// queues both decoder and encoder keys via EpochSignal, resolves the
/// rotation completion oneshot, and produces an audit checkpoint.
#[test]
fn rotate_commit_returns_to_established_with_epoch_keys() {
    let (mut initiator, router_init) = make_test_context();

    let init = initiator.rotation_mut().initiate(30_000, [0xBB; 32]).expect("initiate must succeed");
    initiator.set_session_state(SessionState::Rotating);

    let (mut responder, router_resp) = make_test_context();
    let init_encoded = rotate_codec::encode_init(&init);
    rotate::handle_init(&mut responder, &init_encoded).unwrap();
    let responder_out = responder.drain_outbound();
    let commit_payload = responder_out.iter().find_map(|f| match f {
        OutboundFrame::Channel { kind: ChannelKind::RotateCommit, payload } => Some(payload.clone()),
        _ => None,
    }).expect("responder must produce ROTATE_COMMIT");

    rotate::handle_commit(&mut initiator, &commit_payload).unwrap();

    assert_eq!(initiator.session_state(), SessionState::Established);

    // Epoch signal must have a pending install with both decoder and encoder keys.
    let epoch_install = initiator.epoch_signal().drain();
    assert!(
        epoch_install.is_some(),
        "initiator must have queued epoch keys via install_next_epoch_keys after handle_commit"
    );

    assert_no_router_deliveries(&router_init);
    assert_no_router_deliveries(&router_resp);
}

/// handle_commit produces an AUDIT_CHECKPOINT with the rotation anchor.
#[test]
fn rotate_commit_produces_audit_checkpoint() {
    let (mut initiator, router_init) = make_test_context();

    let init = initiator.rotation_mut().initiate(30_000, [0xBB; 32]).unwrap();
    initiator.set_session_state(SessionState::Rotating);

    let (mut responder, router_resp) = make_test_context();
    rotate::handle_init(&mut responder, &rotate_codec::encode_init(&init)).unwrap();
    let responder_out = responder.drain_outbound();
    let commit_payload = responder_out.iter().find_map(|f| match f {
        OutboundFrame::Channel { kind: ChannelKind::RotateCommit, payload } => Some(payload.clone()),
        _ => None,
    }).unwrap();

    rotate::handle_commit(&mut initiator, &commit_payload).unwrap();
    let out = initiator.drain_outbound();

    assert!(
        out.iter().any(|f| matches!(f, OutboundFrame::Audit { kind: AuditKind::Checkpoint, .. })),
        "rotation commit must produce AUDIT_CHECKPOINT with rotation anchor"
    );
    assert_no_router_deliveries(&router_init);
    assert_no_router_deliveries(&router_resp);
}
