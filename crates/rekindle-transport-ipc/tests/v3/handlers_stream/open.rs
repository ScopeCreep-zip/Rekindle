use rekindle_transport_ipc::v3::dispatch::test_helpers::{make_test_context, assert_no_router_deliveries};
use rekindle_transport_ipc::v3::handlers::stream::open;
use rekindle_transport_ipc::v3::codec::stream::open as open_codec;
use rekindle_transport_ipc::v3::codec::header::StreamHeaderInfo;
use rekindle_transport_ipc::v3::wire::clearance::Clearance;
use rekindle_transport_ipc::v3::wire::frame_class::FrameClass;
use rekindle_transport_ipc::v3::wire::frame_kind::StreamKind;

fn make_header(stream_id: u8) -> StreamHeaderInfo {
    StreamHeaderInfo {
        frame_class: FrameClass::Stream, frame_kind: StreamKind::Open,
        stream_id, header_flags: 0, chunk_index: 0, nonce: 0,
    }
}

fn make_open_payload(stream_id: u8) -> Vec<u8> {
    open_codec::encode(&open_codec::StreamOpenPayload {
        transfer_id: uuid::Uuid::from_u128(stream_id as u128),
        expected_total_bytes: 1_048_576, expected_chunk_count: 64, chunk_size: 16384,
        content_hash: [0xFF; 32], lineage_kind: 0x01, dedup_hint: 0x02,
        clearance_required: Clearance::Public, conditions: vec![],
    })
}

#[test]
fn open_creates_reassembler() {
    let (mut ctx, router) = make_test_context();
    open::handle(&mut ctx, &make_header(5), &make_open_payload(5)).unwrap();
    assert!(ctx.has_reassembler(5));
    assert_no_router_deliveries(&router);
}

#[test]
fn open_registers_stream() {
    let (mut ctx, router) = make_test_context();
    open::handle(&mut ctx, &make_header(7), &make_open_payload(7)).unwrap();
    assert_eq!(ctx.stream_registry().active_count(), 1);
    assert_no_router_deliveries(&router);
}

#[test]
fn open_creates_credit_tracker() {
    let (mut ctx, router) = make_test_context();
    open::handle(&mut ctx, &make_header(3), &make_open_payload(3)).unwrap();
    assert!(ctx.stream_credit_remaining(3) > 0);
    assert_no_router_deliveries(&router);
}

#[test]
fn open_duplicate_stream_id_returns_error() {
    let (mut ctx, router) = make_test_context();
    open::handle(&mut ctx, &make_header(10), &make_open_payload(10)).unwrap();
    let result = open::handle(&mut ctx, &make_header(10), &make_open_payload(10));
    assert!(result.is_err());
    assert_no_router_deliveries(&router);
}

#[test]
fn open_clearance_exceeding_agreed_returns_error() {
    let (mut ctx, router) = make_test_context();
    let payload = open_codec::encode(&open_codec::StreamOpenPayload {
        transfer_id: uuid::Uuid::from_u128(1), expected_total_bytes: 1024,
        expected_chunk_count: 1, chunk_size: 1024, content_hash: [0; 32],
        lineage_kind: 0x01, dedup_hint: 0x03,
        clearance_required: Clearance::Secret, conditions: vec![],
    });
    let result = open::handle(&mut ctx, &make_header(1), &payload);
    assert!(result.is_err());
    assert_no_router_deliveries(&router);
}
