use rekindle_transport_ipc::v3::context::SessionContext;
use rekindle_transport_ipc::v3::dispatch::test_helpers::{make_test_context_with_clearance, assert_no_router_deliveries};
use rekindle_transport_ipc::v3::handlers::stream::open;
use rekindle_transport_ipc::v3::codec::stream::open as open_codec;
use rekindle_transport_ipc::v3::codec::header::StreamHeaderInfo;
use rekindle_transport_ipc::v3::wire::clearance::Clearance;
use rekindle_transport_ipc::v3::wire::frame_class::FrameClass;
use rekindle_transport_ipc::v3::wire::frame_kind::StreamKind;

fn try_open_at_clearance(ctx: &mut SessionContext, clearance: Clearance) -> Result<(), impl std::fmt::Debug> {
    let header = StreamHeaderInfo {
        frame_class: FrameClass::Stream, frame_kind: StreamKind::Open,
        stream_id: 1, header_flags: 0, chunk_index: 0, nonce: 0,
    };
    let payload = open_codec::encode(&open_codec::StreamOpenPayload {
        transfer_id: uuid::Uuid::from_u128(1),
        expected_total_bytes: 0, expected_chunk_count: 0, chunk_size: 0,
        content_hash: [0; 32], lineage_kind: 0x01, dedup_hint: 0x03,
        clearance_required: clearance, conditions: vec![],
    });
    open::handle(ctx, &header, &payload)
}

#[test]
fn internal_peer_cannot_open_secret_stream() {
    let (mut ctx, router) = make_test_context_with_clearance(Clearance::Internal);
    let result = try_open_at_clearance(&mut ctx, Clearance::Secret);
    assert!(result.is_err());
    assert_no_router_deliveries(&router);
}

#[test]
fn internal_peer_cannot_open_confidential_stream() {
    let (mut ctx, router) = make_test_context_with_clearance(Clearance::Internal);
    let result = try_open_at_clearance(&mut ctx, Clearance::Confidential);
    assert!(result.is_err());
    assert_no_router_deliveries(&router);
}

#[test]
fn internal_peer_can_open_internal_stream() {
    let (mut ctx, router) = make_test_context_with_clearance(Clearance::Internal);
    let result = try_open_at_clearance(&mut ctx, Clearance::Internal);
    assert!(result.is_ok());
    assert_no_router_deliveries(&router);
}

#[test]
fn internal_peer_can_open_public_stream() {
    let (mut ctx, router) = make_test_context_with_clearance(Clearance::Internal);
    let result = try_open_at_clearance(&mut ctx, Clearance::Public);
    assert!(result.is_ok());
    assert_no_router_deliveries(&router);
}

#[test]
fn unclassified_peer_cannot_open_public_stream() {
    let (mut ctx, router) = make_test_context_with_clearance(Clearance::Unclassified);
    let result = try_open_at_clearance(&mut ctx, Clearance::Public);
    assert!(result.is_err());
    assert_no_router_deliveries(&router);
}

#[test]
fn top_secret_peer_can_open_any_clearance() {
    for clearance in Clearance::ALL {
        let (mut ctx, router) = make_test_context_with_clearance(Clearance::TopSecret);
        let header = StreamHeaderInfo {
            frame_class: FrameClass::Stream, frame_kind: StreamKind::Open,
            stream_id: clearance as u8, header_flags: 0, chunk_index: 0, nonce: 0,
        };
        let payload = open_codec::encode(&open_codec::StreamOpenPayload {
            transfer_id: uuid::Uuid::from_u128(clearance as u128),
            expected_total_bytes: 0, expected_chunk_count: 0, chunk_size: 0,
            content_hash: [0; 32], lineage_kind: 0x01, dedup_hint: 0x03,
            clearance_required: clearance, conditions: vec![],
        });
        let result = open::handle(&mut ctx, &header, &payload);
        assert!(result.is_ok(), "TopSecret peer must be able to open {clearance:?} stream");
        assert_no_router_deliveries(&router);
    }
}
