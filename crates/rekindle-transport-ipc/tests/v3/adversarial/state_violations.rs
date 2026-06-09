use rekindle_transport_ipc::v3::dispatch::test_helpers::{make_test_context, test_envelope, test_stream_header, assert_no_router_deliveries};
use rekindle_transport_ipc::v3::dispatch::inbound::dispatch_frame;
use rekindle_transport_ipc::v3::handlers::channel::goodbye;
use rekindle_transport_ipc::v3::codec::stream::open as open_codec;
use rekindle_transport_ipc::v3::codec::channel::goodbye as goodbye_codec;
use rekindle_transport_ipc::v3::session::state::{SessionEvent, SessionState};
use rekindle_transport_ipc::v3::wire::clearance::Clearance;
use rekindle_transport_ipc::v3::wire::frame_class::FrameClass;
use rekindle_transport_ipc::v3::wire::frame_kind::StreamKind;
use rekindle_transport_ipc::v3::wire::lane::Lane;

/// STREAM_OPEN is rejected during Rotating via dispatch_frame's is_frame_allowed.
/// A new stream's lifecycle would span the key boundary. In-flight payload/ack/fin
/// from existing streams ARE allowed (tested in frame_validity.rs).
#[test]
fn stream_open_while_rotating_rejected() {
    let (mut ctx, router) = make_test_context();
    ctx.session_state_mut()
        .apply(SessionEvent::RotateInitSent)
        .expect("transition to Rotating");
    assert_eq!(ctx.session_state(), SessionState::Rotating);

    let envelope = test_envelope(Lane::Data);
    let header = test_stream_header(StreamKind::Open);
    let mut plaintext = vec![FrameClass::Stream as u8, StreamKind::Open as u8];
    plaintext.extend_from_slice(&open_codec::encode(&open_codec::StreamOpenPayload {
        transfer_id: uuid::Uuid::from_u128(1),
        expected_total_bytes: 0, expected_chunk_count: 0, chunk_size: 0,
        content_hash: [0; 32], lineage_kind: 0x01, dedup_hint: 0x03,
        clearance_required: Clearance::Public, conditions: vec![],
    }));

    let result = dispatch_frame(&mut ctx, &envelope, Some(&header), &plaintext);
    assert!(result.is_err(), "STREAM_OPEN must be rejected while Rotating");
    assert_no_router_deliveries(&router);
}

/// Stream open while Draining is REJECTED via dispatch_frame's is_frame_allowed.
/// The handler itself does not check session state — dispatch_frame is the SSOT
/// for frame admission. This test exercises the full receive path.
#[test]
fn stream_open_while_draining_rejected() {
    let (mut ctx, router) = make_test_context();
    goodbye::handle(&mut ctx, &goodbye_codec::encode(&goodbye_codec::GoodbyePayload {
        reason_code: 0, drain_timeout_ms: 30_000, final_session_seq: 0,
    })).unwrap();
    assert_eq!(ctx.session_state(), SessionState::Draining);
    ctx.drain_outbound();

    let envelope = test_envelope(Lane::Data);
    let header = test_stream_header(StreamKind::Open);
    let mut plaintext = vec![FrameClass::Stream as u8, StreamKind::Open as u8];
    plaintext.extend_from_slice(&open_codec::encode(&open_codec::StreamOpenPayload {
        transfer_id: uuid::Uuid::from_u128(1),
        expected_total_bytes: 0, expected_chunk_count: 0, chunk_size: 0,
        content_hash: [0; 32], lineage_kind: 0x01, dedup_hint: 0x03,
        clearance_required: Clearance::Public, conditions: vec![],
    }));

    let result = dispatch_frame(&mut ctx, &envelope, Some(&header), &plaintext);
    assert!(result.is_err(), "STREAM_OPEN must be rejected while Draining");
    assert_no_router_deliveries(&router);
}

/// Stream open while Quiesced is REJECTED via dispatch_frame's is_frame_allowed.
#[test]
fn stream_open_while_quiesced_rejected() {
    let (mut ctx, router) = make_test_context();
    use rekindle_transport_ipc::v3::handlers::channel::quiesce;
    use rekindle_transport_ipc::v3::codec::channel::quiesce as quiesce_codec;
    let qp = quiesce_codec::encode(&quiesce_codec::QuiescePayload {
        duration_ms: 60_000, reason_code: 0,
    });
    quiesce::handle_quiesce(&mut ctx, &qp).unwrap();
    assert_eq!(ctx.session_state(), SessionState::Quiesced);
    ctx.drain_outbound();

    let envelope = test_envelope(Lane::Data);
    let header = test_stream_header(StreamKind::Open);
    let mut plaintext = vec![FrameClass::Stream as u8, StreamKind::Open as u8];
    plaintext.extend_from_slice(&open_codec::encode(&open_codec::StreamOpenPayload {
        transfer_id: uuid::Uuid::from_u128(1),
        expected_total_bytes: 0, expected_chunk_count: 0, chunk_size: 0,
        content_hash: [0; 32], lineage_kind: 0x01, dedup_hint: 0x03,
        clearance_required: Clearance::Public, conditions: vec![],
    }));

    let result = dispatch_frame(&mut ctx, &envelope, Some(&header), &plaintext);
    assert!(result.is_err(), "STREAM_OPEN must be rejected while Quiesced");
    assert_no_router_deliveries(&router);
}
