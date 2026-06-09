use rekindle_transport_ipc::v3::context::OutboundFrame;
use rekindle_transport_ipc::v3::dispatch::test_helpers::{make_test_context, assert_no_router_deliveries};
use rekindle_transport_ipc::v3::handlers::stream::cancel;
use rekindle_transport_ipc::v3::codec::stream::cancel as cancel_codec;
use rekindle_transport_ipc::v3::codec::header::StreamHeaderInfo;
use rekindle_transport_ipc::v3::wire::frame_class::FrameClass;
use rekindle_transport_ipc::v3::wire::frame_kind::StreamKind;

#[test]
fn cancel_produces_cancel_ack() {
    let (mut ctx, router) = make_test_context();
    ctx.stream_registry_mut().open(5).unwrap();
    ctx.create_reassembler(5);

    let header = StreamHeaderInfo {
        frame_class: FrameClass::Stream, frame_kind: StreamKind::Cancel,
        stream_id: 5, header_flags: 0, chunk_index: 0, nonce: 1,
    };
    let payload = cancel_codec::encode(&cancel_codec::StreamCancelPayload {
        transfer_id: uuid::Uuid::from_u128(5), bytes_through: 500_000, chunks_through: 3,
    });
    cancel::handle(&mut ctx, &header, &payload).unwrap();

    let out = ctx.drain_outbound();
    assert!(
        out.iter().any(|f| matches!(f, OutboundFrame::Data { kind: StreamKind::CancelAck, .. })),
        "cancel must produce CANCEL_ACK"
    );
    assert_no_router_deliveries(&router);
}

#[test]
fn cancel_registers_in_resume_registry() {
    let (mut ctx, router) = make_test_context();
    let tid = uuid::Uuid::from_u128(7);
    ctx.stream_registry_mut().open(7).unwrap();
    ctx.create_reassembler(7);

    let header = StreamHeaderInfo {
        frame_class: FrameClass::Stream, frame_kind: StreamKind::Cancel,
        stream_id: 7, header_flags: 0, chunk_index: 0, nonce: 1,
    };
    let payload = cancel_codec::encode(&cancel_codec::StreamCancelPayload {
        transfer_id: tid, bytes_through: 0, chunks_through: 0,
    });
    cancel::handle(&mut ctx, &header, &payload).unwrap();

    assert!(ctx.has_resume_state(tid), "cancel must register in resume registry for later resume");
    assert_no_router_deliveries(&router);
}

#[test]
fn cancel_clears_reassembler() {
    let (mut ctx, router) = make_test_context();
    ctx.stream_registry_mut().open(3).unwrap();
    ctx.create_reassembler(3);
    assert!(ctx.has_reassembler(3));

    let header = StreamHeaderInfo {
        frame_class: FrameClass::Stream, frame_kind: StreamKind::Cancel,
        stream_id: 3, header_flags: 0, chunk_index: 0, nonce: 1,
    };
    let payload = cancel_codec::encode(&cancel_codec::StreamCancelPayload {
        transfer_id: uuid::Uuid::from_u128(3), bytes_through: 0, chunks_through: 0,
    });
    cancel::handle(&mut ctx, &header, &payload).unwrap();
    assert!(!ctx.has_reassembler(3));
    assert_no_router_deliveries(&router);
}
