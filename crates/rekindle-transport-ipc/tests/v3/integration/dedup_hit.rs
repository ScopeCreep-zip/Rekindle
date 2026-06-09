use rekindle_transport_ipc::v3::context::OutboundFrame;
use rekindle_transport_ipc::v3::dispatch::test_helpers::make_test_context;
use rekindle_transport_ipc::v3::handlers::stream::reference;
use rekindle_transport_ipc::v3::codec::stream::reference as ref_codec;
use rekindle_transport_ipc::v3::codec::header::StreamHeaderInfo;
use rekindle_transport_ipc::v3::wire::clearance::Clearance;
use rekindle_transport_ipc::v3::wire::frame_class::FrameClass;
use rekindle_transport_ipc::v3::wire::frame_kind::StreamKind;

#[test]
fn dedup_cache_hit_skips_retransmission() {
    let (mut ctx, router) = make_test_context();
    let content = vec![0xDE; 10_000];
    let hash = *blake3::hash(&content).as_bytes();

    // Pre-populate the receiver cache (simulating a previous transfer)
    ctx.receiver_cache_mut().store(hash, content.clone(), content.len() as u64, 1, Clearance::Internal);

    // Send STREAM_REFERENCE instead of full stream
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

    // Should produce STREAM_ACK -- the transfer is complete from cache
    let out = ctx.drain_outbound();
    assert!(
        out.iter().any(|f| matches!(f, OutboundFrame::Data { kind: StreamKind::Ack, .. })),
        "dedup cache hit must produce STREAM_ACK without any PAYLOAD frames"
    );

    // No bulk deliveries should be staged — cache hit means no payload frames
    let deliveries = ctx.drain_bulk_deliveries();
    assert!(deliveries.is_empty(), "dedup cache hit must not produce any bulk deliveries");

    // No lifecycle events — dedup reference is a single-frame shortcut
    assert_eq!(router.bulk_completes.lock().len(), 0, "dedup reference must not fire on_bulk_complete");
    assert_eq!(router.bulk_failures.lock().len(), 0, "dedup reference must not fire on_bulk_failed");
}
