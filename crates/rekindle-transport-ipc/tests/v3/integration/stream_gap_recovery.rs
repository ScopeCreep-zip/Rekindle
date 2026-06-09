use rekindle_transport_ipc::v3::context::OutboundFrame;
use rekindle_transport_ipc::v3::dispatch::test_helpers::{
    make_test_context, assert_no_router_deliveries, assert_bulk_delivered,
};
use rekindle_transport_ipc::v3::handlers::stream::{open, payload};
use rekindle_transport_ipc::v3::handlers::audit::gap;
use rekindle_transport_ipc::v3::codec::stream::open as open_codec;
use rekindle_transport_ipc::v3::codec::audit::gap as gap_codec;
use rekindle_transport_ipc::v3::codec::header::StreamHeaderInfo;
use rekindle_transport_ipc::v3::wire::clearance::Clearance;
use rekindle_transport_ipc::v3::wire::frame_class::FrameClass;
use rekindle_transport_ipc::v3::wire::frame_kind::StreamKind;

#[test]
fn gap_in_stream_detected_by_reassembler() {
    let (mut ctx, router) = make_test_context();

    let open_header = StreamHeaderInfo {
        frame_class: FrameClass::Stream, frame_kind: StreamKind::Open,
        stream_id: 1, header_flags: 0, chunk_index: 0, nonce: 0,
    };
    open::handle(&mut ctx, &open_header, &open_codec::encode(&open_codec::StreamOpenPayload {
        transfer_id: uuid::Uuid::from_u128(1),
        expected_total_bytes: 500,
        expected_chunk_count: 5,
        chunk_size: 100,
        content_hash: [0; 32],
        lineage_kind: 0x01, dedup_hint: 0x03,
        clearance_required: Clearance::Public, conditions: vec![],
    })).unwrap();

    // Send chunks 0, 1, 2, 4 (skip 3)
    for i in [0u32, 1, 2, 4] {
        let header = StreamHeaderInfo {
            frame_class: FrameClass::Stream, frame_kind: StreamKind::Payload,
            stream_id: 1, header_flags: 0, chunk_index: i, nonce: i as u64 + 1,
        };
        payload::handle(&mut ctx, &header, &vec![i as u8; 100]).unwrap();
    }

    assert_eq!(ctx.reassembler_next_expected(1), 3);
    assert!(ctx.reassembler_buffered_count(1) >= 1);

    // Verify that only in-order chunks (0, 1, 2) were delivered.
    // Chunk 4 is buffered because chunk 3 hasn't arrived yet.
    assert_bulk_delivered(&mut ctx, 1, &[&vec![0u8; 100], &vec![1u8; 100], &vec![2u8; 100]]);
    assert_no_router_deliveries(&router);
}

#[test]
fn gap_handler_produces_replay_from_retained_frames() {
    let (mut ctx, router) = make_test_context();

    // Store frames in retention (simulating outbound frames we sent)
    for seq in 0..10u64 {
        ctx.retention_mut().store(seq, vec![seq as u8; 100]);
    }

    // Peer sends AUDIT_GAP requesting retransmission of seqs 3 and 5
    let gap_payload = gap_codec::encode(&gap_codec::AuditGapPayload {
        gap_id: uuid::Uuid::from_u128(1),
        gap_start_seq: 3,
        gap_end_seq: 5,
        gap_detected_ns: 0,
        expected_chain_link: [0; 32],
        missing_bitmap: vec![0b0000_0101], // bits 0 and 2 set = seqs 3 and 5
    });
    gap::handle(&mut ctx, &gap_payload).unwrap();

    let out = ctx.drain_outbound();
    assert!(
        out.iter().any(|f| matches!(f, OutboundFrame::Audit { kind: rekindle_transport_ipc::v3::wire::frame_kind::AuditKind::Replay, .. })),
        "gap handler must produce AUDIT_REPLAY"
    );
    assert_no_router_deliveries(&router);
}
