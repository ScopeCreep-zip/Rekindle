use rekindle_transport_ipc::v3::codec::header::StreamHeaderInfo;
use rekindle_transport_ipc::v3::dispatch::test_helpers::{make_test_context, assert_no_router_deliveries};
use rekindle_transport_ipc::v3::handlers::stream::reset;
use rekindle_transport_ipc::v3::wire::failure::FailureCode;
use rekindle_transport_ipc::v3::wire::frame_class::FrameClass;
use rekindle_transport_ipc::v3::wire::frame_kind::StreamKind;

#[test]
fn reset_clears_reassembler_and_releases_stream() {
    let (mut ctx, router) = make_test_context();
    ctx.open_inbound_stream(5).unwrap();
    ctx.create_reassembler(5);

    let header = StreamHeaderInfo {
        frame_class: FrameClass::Stream, frame_kind: StreamKind::Reset,
        stream_id: 5, header_flags: 0, chunk_index: 0, nonce: 1,
    };
    let mut payload = vec![0u8; 8];
    payload[0..4].copy_from_slice(&(FailureCode::PoolExhausted as u32).to_le_bytes());
    reset::handle(&mut ctx, &header, &payload).unwrap();

    assert!(!ctx.has_reassembler(5));
    assert_eq!(ctx.stream_active_count(), 0);
    assert_no_router_deliveries(&router);
}
