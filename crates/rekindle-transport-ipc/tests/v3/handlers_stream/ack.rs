use rekindle_transport_ipc::v3::codec::header::StreamHeaderInfo;
use rekindle_transport_ipc::v3::codec::stream::ack as ack_codec;
use rekindle_transport_ipc::v3::dispatch::test_helpers::make_test_context;
use rekindle_transport_ipc::v3::handlers::stream::ack;
use rekindle_transport_ipc::v3::stream::state::{StreamEvent, StreamState};
use rekindle_transport_ipc::v3::wire::frame_class::FrameClass;
use rekindle_transport_ipc::v3::wire::frame_kind::StreamKind;

#[test]
fn ack_closes_stream_and_releases_id() {
    let (mut ctx, router) = make_test_context();
    ctx.open_outbound_stream(5).unwrap();
    ctx.transition_outbound_stream(5, StreamEvent::FinSent).unwrap();

    let transfer_id = uuid::Uuid::from_u128(5);
    let header = StreamHeaderInfo {
        frame_class: FrameClass::Stream, frame_kind: StreamKind::Ack,
        stream_id: 5, header_flags: 0, chunk_index: 10, nonce: 1,
    };
    let payload = ack_codec::encode(&ack_codec::StreamAckPayload {
        transfer_id,
        ack_byte_count: 1_048_576, ack_chunk_count: 10, audit_link: [0; 32],
    });
    ack::handle(&mut ctx, &header, &payload).unwrap();

    assert_eq!(ctx.stream_active_count(), 0);
    assert_eq!(ctx.outbound_stream_state(5), Some(StreamState::Closed));

    let completes = router.bulk_completes.lock();
    assert_eq!(completes.len(), 1, "ack handler must deliver bulk_complete to router");
    assert_eq!(completes[0].stream_id, 5);
    assert_eq!(completes[0].transfer_id, transfer_id);
    assert_eq!(completes[0].total_bytes, 1_048_576);
    assert_eq!(completes[0].total_chunks, 10);
}
