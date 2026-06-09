use rekindle_transport_ipc::v3::context::OutboundFrame;
use rekindle_transport_ipc::v3::dispatch::test_helpers::{make_test_context, assert_no_router_deliveries};
use rekindle_transport_ipc::v3::handlers::audit::checkpoint;
use rekindle_transport_ipc::v3::codec::audit::checkpoint as cp_codec;
use rekindle_transport_ipc::v3::session::state::SessionState;
use rekindle_transport_ipc::v3::wire::frame_kind::ChannelKind;

#[test]
fn valid_checkpoint_accepted() {
    let (mut ctx, router) = make_test_context();
    let cp = cp_codec::AuditCheckpointPayload {
        chain_index: 0, chain_length: 0, checkpoint_seq: 0, wall_clock_ns: 0,
        chain_link: ctx.inbound_chain().current_link(),
        anchor_link: ctx.inbound_chain().anchor_record().value,
    };
    let payload = cp_codec::encode(&cp);
    let result = checkpoint::handle(&mut ctx, &payload);
    assert!(result.is_ok());
    assert_no_router_deliveries(&router);
}

#[test]
fn checkpoint_mismatch_produces_channel_error() {
    let (mut ctx, router) = make_test_context();
    let cp = cp_codec::AuditCheckpointPayload {
        chain_index: 100, chain_length: 100, checkpoint_seq: 1, wall_clock_ns: 0,
        chain_link: [0xFF; 32], anchor_link: [0; 32],
    };
    let payload = cp_codec::encode(&cp);
    let result = checkpoint::handle(&mut ctx, &payload);
    let out = ctx.drain_outbound();
    let has_error = result.is_err() || out.iter().any(|f| matches!(f, OutboundFrame::Channel { kind: ChannelKind::ChannelError, .. }));
    assert!(has_error, "mismatched checkpoint must produce error");

    assert!(
        out.iter().any(|f| matches!(f, OutboundFrame::Channel { kind: ChannelKind::ChannelError, .. })),
        "checkpoint mismatch must emit CHANNEL_ERROR for dispatch to close session"
    );

    assert_eq!(ctx.session_state(), SessionState::Established);
    assert_no_router_deliveries(&router);
}
