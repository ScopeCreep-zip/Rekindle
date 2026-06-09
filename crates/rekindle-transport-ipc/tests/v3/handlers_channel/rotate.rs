use rekindle_transport_ipc::v3::context::OutboundFrame;
use rekindle_transport_ipc::v3::dispatch::test_helpers::{make_test_context, assert_no_router_deliveries};
use rekindle_transport_ipc::v3::handlers::channel::rotate;
use rekindle_transport_ipc::v3::codec::channel::rotate as rotate_codec;
use rekindle_transport_ipc::v3::session::state::SessionState;
use rekindle_transport_ipc::v3::wire::frame_kind::{AuditKind, ChannelKind};

/// handle_init (responder path): receives ROTATE_INIT, produces ROTATE_COMMIT,
/// installs decoder keys, stores deferred encoder keys, and transitions
/// Established → Rotating → Established (via RotateCommitSent).
///
/// The responder completes its rotation handshake in a single handler call.
/// The session returns to Established immediately — the responder's encoder
/// stays on epoch=0 until the initiator's first epoch=1 frame arrives.
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

    // Deferred encoder keys stored — the control loop installs them
    // when peer_epoch_advanced is detected.
    assert!(
        responder.take_deferred_encoder().is_some(),
        "responder must have deferred encoder keys after handle_init"
    );

    assert_no_router_deliveries(&router);
}

/// handle_commit (initiator path): receives ROTATE_COMMIT, derives new keys,
/// installs decoder keys, stores immediate encoder keys, resolves the
/// rotation completion oneshot, and produces an audit checkpoint.
///
/// The initiator's ctx.keys() is NOT updated — keys go to the encoder/decoder
/// via EpochSignal. The rotation coordinator (ctx.rotation_mut().current_keys())
/// holds the new DerivedKeys.
#[test]
fn rotate_commit_returns_to_established_with_pending_encoder() {
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

    // Immediate encoder keys stored — the control loop installs them
    // right after drain_outbound on the same select! iteration.
    assert!(
        initiator.take_immediate_encoder().is_some(),
        "initiator must have immediate encoder keys after handle_commit"
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
