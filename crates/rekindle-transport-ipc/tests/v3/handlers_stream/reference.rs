use rekindle_transport_ipc::v3::context::OutboundFrame;
use rekindle_transport_ipc::v3::dispatch::test_helpers::{make_test_context, assert_no_router_deliveries};
use rekindle_transport_ipc::v3::handlers::stream::reference;
use rekindle_transport_ipc::v3::codec::stream::reference as ref_codec;
use rekindle_transport_ipc::v3::codec::header::StreamHeaderInfo;
use rekindle_transport_ipc::v3::wire::clearance::Clearance;
use rekindle_transport_ipc::v3::wire::frame_class::FrameClass;
use rekindle_transport_ipc::v3::wire::frame_kind::StreamKind;

#[test]
fn reference_cache_hit_produces_ack() {
    let (mut ctx, router) = make_test_context();
    let content = b"cached content for dedup test";
    let hash = *blake3::hash(content).as_bytes();
    ctx.receiver_cache_mut().store(hash, content.to_vec(), content.len() as u64, 1, Clearance::Internal);

    let header = StreamHeaderInfo {
        frame_class: FrameClass::Stream, frame_kind: StreamKind::Reference,
        stream_id: 5, header_flags: 0, chunk_index: 0, nonce: 1,
    };
    let payload = ref_codec::encode(&ref_codec::StreamReferencePayload {
        transfer_id: uuid::Uuid::from_u128(5),
        content_hash: hash,
        expected_total_bytes: content.len() as u64,
        expected_chunk_count: 1,
        reference_kind: 0x01,
        sender_clearance: Clearance::Internal,
        prefix_byte_count: 0,
        prefix_content_hash: [0; 32],
    });
    reference::handle(&mut ctx, &header, &payload).unwrap();

    let out = ctx.drain_outbound();
    assert!(
        out.iter().any(|f| matches!(f, OutboundFrame::Data { kind: StreamKind::Ack, .. })),
        "cache hit must produce STREAM_ACK"
    );
    assert_no_router_deliveries(&router);
}

#[test]
fn reference_cache_miss_produces_nack() {
    let (mut ctx, router) = make_test_context();
    let unknown_hash = [0xFF; 32];

    let header = StreamHeaderInfo {
        frame_class: FrameClass::Stream, frame_kind: StreamKind::Reference,
        stream_id: 5, header_flags: 0, chunk_index: 0, nonce: 1,
    };
    let payload = ref_codec::encode(&ref_codec::StreamReferencePayload {
        transfer_id: uuid::Uuid::from_u128(5),
        content_hash: unknown_hash,
        expected_total_bytes: 1024,
        expected_chunk_count: 1,
        reference_kind: 0x01,
        sender_clearance: Clearance::Internal,
        prefix_byte_count: 0,
        prefix_content_hash: [0; 32],
    });
    let result = reference::handle(&mut ctx, &header, &payload);
    assert!(result.is_err());
    assert_no_router_deliveries(&router);
}
