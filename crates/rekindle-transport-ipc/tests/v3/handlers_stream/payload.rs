use rekindle_transport_ipc::v3::context::SessionContext;
use rekindle_transport_ipc::v3::dispatch::test_helpers::{
    make_test_context, assert_bulk_delivered, assert_no_bulk_deliveries,
    assert_no_router_deliveries,
};
use rekindle_transport_ipc::v3::handlers::stream::open;
use rekindle_transport_ipc::v3::handlers::stream::payload;
use rekindle_transport_ipc::v3::codec::stream::open as open_codec;
use rekindle_transport_ipc::v3::codec::header::StreamHeaderInfo;
use rekindle_transport_ipc::v3::wire::clearance::Clearance;
use rekindle_transport_ipc::v3::wire::frame_class::FrameClass;
use rekindle_transport_ipc::v3::wire::frame_kind::StreamKind;

fn open_stream(ctx: &mut SessionContext, stream_id: u8) {
    let header = StreamHeaderInfo {
        frame_class: FrameClass::Stream,
        frame_kind: StreamKind::Open,
        stream_id,
        header_flags: 0,
        chunk_index: 0,
        nonce: 0,
    };
    let payload = open_codec::encode(&open_codec::StreamOpenPayload {
        transfer_id: uuid::Uuid::from_u128(stream_id as u128),
        expected_total_bytes: 1_048_576,
        expected_chunk_count: 64,
        chunk_size: 16384,
        content_hash: [0; 32],
        lineage_kind: 0x01,
        dedup_hint: 0x03,
        clearance_required: Clearance::Public,
        conditions: vec![],
    });
    open::handle(ctx, &header, &payload).unwrap();
}

fn make_payload_header(stream_id: u8, chunk_index: u32, nonce: u64) -> StreamHeaderInfo {
    StreamHeaderInfo {
        frame_class: FrameClass::Stream,
        frame_kind: StreamKind::Payload,
        stream_id,
        header_flags: 0,
        chunk_index,
        nonce,
    }
}

#[test]
fn payload_delivers_to_reassembler() {
    let (mut ctx, router) = make_test_context();
    open_stream(&mut ctx, 5);
    let header = make_payload_header(5, 0, 1);
    let data = b"hello world";
    payload::handle(&mut ctx, &header, data).unwrap();
    assert_eq!(ctx.reassembler_next_expected(5), 1);
    assert_bulk_delivered(&mut ctx, 5, &[b"hello world"]);
    assert_no_router_deliveries(&router);
}

#[test]
fn payload_handler_does_not_advance_audit_chain() {
    let (mut ctx, router) = make_test_context();
    let chain_len_before = ctx.inbound_chain().length();
    open_stream(&mut ctx, 3);
    let header = make_payload_header(3, 0, 1);
    payload::handle(&mut ctx, &header, b"data").unwrap();
    assert_eq!(
        ctx.inbound_chain().length(), chain_len_before,
        "handler must NOT advance audit chain — that's the control loop's job"
    );
    assert_bulk_delivered(&mut ctx, 3, &[b"data"]);
    assert_no_router_deliveries(&router);
}

#[test]
fn payload_for_unknown_stream_returns_error() {
    let (mut ctx, router) = make_test_context();
    let header = make_payload_header(99, 0, 1);
    let result = payload::handle(&mut ctx, &header, b"data");
    assert!(result.is_err());
    assert_no_bulk_deliveries(&mut ctx);
    assert_no_router_deliveries(&router);
}

#[test]
fn multiple_chunks_in_order() {
    let (mut ctx, router) = make_test_context();
    open_stream(&mut ctx, 1);
    let mut expected: Vec<Vec<u8>> = Vec::new();
    for i in 0..10u32 {
        let header = make_payload_header(1, i, i as u64 + 1);
        let data = vec![i as u8; 100];
        payload::handle(&mut ctx, &header, &data).unwrap();
        expected.push(data);
    }
    assert_eq!(ctx.reassembler_next_expected(1), 10);

    let expected_refs: Vec<&[u8]> = expected.iter().map(|v| v.as_slice()).collect();
    assert_bulk_delivered(&mut ctx, 1, &expected_refs);
    assert_no_router_deliveries(&router);
}
