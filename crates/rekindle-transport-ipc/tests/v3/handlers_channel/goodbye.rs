use rekindle_transport_ipc::v3::context::OutboundFrame;
use rekindle_transport_ipc::v3::wire::frame_kind::ChannelKind;
use rekindle_transport_ipc::v3::dispatch::test_helpers::{make_test_context, assert_no_router_deliveries};
use rekindle_transport_ipc::v3::handlers::channel::goodbye;
use rekindle_transport_ipc::v3::handlers::channel::goodbye_ack;
use rekindle_transport_ipc::v3::codec::channel::goodbye as goodbye_codec;
use rekindle_transport_ipc::v3::session::state::SessionState;

#[test]
fn goodbye_transitions_to_draining() {
    let (mut ctx, router) = make_test_context();
    let payload = goodbye_codec::encode(&goodbye_codec::GoodbyePayload {
        reason_code: 0, drain_timeout_ms: 30_000, final_session_seq: 100,
    });
    goodbye::handle(&mut ctx, &payload).unwrap();
    assert_eq!(ctx.session_state(), SessionState::Draining);
    assert_no_router_deliveries(&router);
}

#[test]
fn goodbye_produces_goodbye_ack() {
    let (mut ctx, router) = make_test_context();
    let payload = goodbye_codec::encode(&goodbye_codec::GoodbyePayload {
        reason_code: 0, drain_timeout_ms: 30_000, final_session_seq: 100,
    });
    goodbye::handle(&mut ctx, &payload).unwrap();
    let out = ctx.drain_outbound();
    assert!(
        out.iter().any(|f| matches!(f, OutboundFrame::Channel { kind: ChannelKind::GoodbyeAck, .. })),
        "goodbye must produce GOODBYE_ACK"
    );
    assert_no_router_deliveries(&router);
}

#[test]
fn goodbye_records_final_session_seq() {
    let (mut ctx, router) = make_test_context();
    let payload = goodbye_codec::encode(&goodbye_codec::GoodbyePayload {
        reason_code: 0, drain_timeout_ms: 30_000, final_session_seq: 9999,
    });
    goodbye::handle(&mut ctx, &payload).unwrap();
    assert_eq!(ctx.peer_final_session_seq(), Some(9999));
    assert_no_router_deliveries(&router);
}

#[test]
fn goodbye_ack_after_both_directions_transitions_to_closed() {
    let (mut ctx, router) = make_test_context();
    let payload = goodbye_codec::encode(&goodbye_codec::GoodbyePayload {
        reason_code: 0, drain_timeout_ms: 30_000, final_session_seq: 100,
    });
    goodbye::handle(&mut ctx, &payload).unwrap();
    assert_eq!(ctx.session_state(), SessionState::Draining);
    ctx.drain_outbound();

    ctx.mark_local_goodbye_sent();

    let ack_payload = goodbye_codec::encode(&goodbye_codec::GoodbyePayload {
        reason_code: 0, drain_timeout_ms: 0, final_session_seq: 0,
    });
    goodbye_ack::handle(&mut ctx, &ack_payload).unwrap();
    assert_eq!(ctx.session_state(), SessionState::Closed);
    assert_no_router_deliveries(&router);
}
