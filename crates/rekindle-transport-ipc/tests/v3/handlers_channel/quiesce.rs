use rekindle_transport_ipc::v3::context::OutboundFrame;
use rekindle_transport_ipc::v3::wire::frame_kind::ChannelKind;
use rekindle_transport_ipc::v3::dispatch::test_helpers::{make_test_context, assert_no_router_deliveries};
use rekindle_transport_ipc::v3::handlers::channel::quiesce;
use rekindle_transport_ipc::v3::codec::channel::quiesce as quiesce_codec;
use rekindle_transport_ipc::v3::session::state::SessionState;

#[test]
fn quiesce_transitions_to_quiesced() {
    let (mut ctx, router) = make_test_context();
    let payload = quiesce_codec::encode(&quiesce_codec::QuiescePayload {
        duration_ms: 60_000, reason_code: 0,
    });
    quiesce::handle_quiesce(&mut ctx, &payload).unwrap();
    assert_eq!(ctx.session_state(), SessionState::Quiesced);
    assert!(ctx.quiescence_deadline().is_some());
    assert_no_router_deliveries(&router);
}

#[test]
fn quiesce_produces_quiesce_ack() {
    let (mut ctx, router) = make_test_context();
    let payload = quiesce_codec::encode(&quiesce_codec::QuiescePayload {
        duration_ms: 60_000, reason_code: 0,
    });
    quiesce::handle_quiesce(&mut ctx, &payload).unwrap();
    let out = ctx.drain_outbound();
    assert!(out.iter().any(|f| matches!(f, OutboundFrame::Channel { kind: ChannelKind::QuiesceAck, .. })));
    assert_no_router_deliveries(&router);
}

#[test]
fn resume_transitions_back_to_established() {
    let (mut ctx, router) = make_test_context();
    let payload = quiesce_codec::encode(&quiesce_codec::QuiescePayload {
        duration_ms: 60_000, reason_code: 0,
    });
    quiesce::handle_quiesce(&mut ctx, &payload).unwrap();
    ctx.drain_outbound();
    quiesce::handle_resume(&mut ctx, &[]).unwrap();
    assert_eq!(ctx.session_state(), SessionState::Established);
    assert!(ctx.quiescence_deadline().is_none());
    assert_no_router_deliveries(&router);
}

#[test]
fn resume_produces_resume_ack() {
    let (mut ctx, router) = make_test_context();
    let payload = quiesce_codec::encode(&quiesce_codec::QuiescePayload {
        duration_ms: 60_000, reason_code: 0,
    });
    quiesce::handle_quiesce(&mut ctx, &payload).unwrap();
    ctx.drain_outbound();
    quiesce::handle_resume(&mut ctx, &[]).unwrap();
    let out = ctx.drain_outbound();
    assert!(out.iter().any(|f| matches!(f, OutboundFrame::Channel { kind: ChannelKind::ResumeAck, .. })));
    assert_no_router_deliveries(&router);
}
