use rekindle_transport_ipc::v3::context::SessionConfig;
use rekindle_transport_ipc::v3::dispatch::test_helpers::{make_test_context, make_test_context_with_config, assert_no_router_deliveries};
use rekindle_transport_ipc::v3::handlers::stream::open;
use rekindle_transport_ipc::v3::handlers::datagram::request;
use rekindle_transport_ipc::v3::codec::stream::open as open_codec;
use rekindle_transport_ipc::v3::codec::datagram::request as req_codec;
use rekindle_transport_ipc::v3::codec::header::StreamHeaderInfo;
use rekindle_transport_ipc::v3::wire::clearance::Clearance;
use rekindle_transport_ipc::v3::wire::frame_class::FrameClass;
use rekindle_transport_ipc::v3::wire::frame_kind::StreamKind;

#[test]
fn stream_open_beyond_256_rejected() {
    let (mut ctx, router) = make_test_context();

    for id in 0..=255u8 {
        let header = StreamHeaderInfo {
            frame_class: FrameClass::Stream, frame_kind: StreamKind::Open,
            stream_id: id, header_flags: 0, chunk_index: 0, nonce: 0,
        };
        let payload = open_codec::encode(&open_codec::StreamOpenPayload {
            transfer_id: uuid::Uuid::from_u128(id as u128),
            expected_total_bytes: 0, expected_chunk_count: 0, chunk_size: 0,
            content_hash: [0; 32], lineage_kind: 0x01, dedup_hint: 0x03,
            clearance_required: Clearance::Public, conditions: vec![],
        });
        open::handle(&mut ctx, &header, &payload).unwrap();
    }
    assert_eq!(ctx.stream_registry().active_count(), 256);

    let header = StreamHeaderInfo {
        frame_class: FrameClass::Stream, frame_kind: StreamKind::Open,
        stream_id: 0, header_flags: 0, chunk_index: 0, nonce: 0,
    };
    let payload = open_codec::encode(&open_codec::StreamOpenPayload {
        transfer_id: uuid::Uuid::from_u128(999),
        expected_total_bytes: 0, expected_chunk_count: 0, chunk_size: 0,
        content_hash: [0; 32], lineage_kind: 0x01, dedup_hint: 0x03,
        clearance_required: Clearance::Public, conditions: vec![],
    });
    let result = open::handle(&mut ctx, &header, &payload);
    assert!(result.is_err(), "reusing open stream_id must be rejected");
    assert_no_router_deliveries(&router);
}

#[test]
fn pending_requests_overflow_rejected() {
    let (mut ctx, router) = make_test_context_with_config(SessionConfig {
        max_pending_requests: 2,
        max_subscriptions: 100,
        ..SessionConfig::default()
    });

    // Fill pending requests to capacity via handler.
    // Slots are held until the control loop encodes the application's
    // DATAGRAM_REPLY (drain_outbound resolves by correlation_id).
    // In this unit test, no control loop runs — slots accumulate.
    for i in 0..2u128 {
        let payload = req_codec::encode(&req_codec::DatagramRequestPayload {
            message_id: uuid::Uuid::from_u128(i),
            reply_timeout_ms: 5000,
            sender_clearance: Clearance::Internal,
            application_payload: vec![],
            conditions: vec![],
        });
        request::handle(&mut ctx, &payload).unwrap();
    }
    assert_eq!(ctx.pending_request_count(), 2);

    // Verify the 2 successful requests were delivered to router
    let requests = router.requests.lock();
    assert_eq!(requests.len(), 2, "2 successful requests must be delivered to router");
    drop(requests);

    // Third request must fail — slots held, limit reached
    let payload = req_codec::encode(&req_codec::DatagramRequestPayload {
        message_id: uuid::Uuid::from_u128(999),
        reply_timeout_ms: 5000,
        sender_clearance: Clearance::Internal,
        application_payload: vec![],
        conditions: vec![],
    });
    let result = request::handle(&mut ctx, &payload);
    assert!(result.is_err(), "pending request beyond max must be rejected by handler");

    // Still only 2 requests in router — overflow was rejected before routing
    let requests = router.requests.lock();
    assert_eq!(requests.len(), 2, "overflow request must not reach router");
}

#[test]
fn subscription_quota_enforced() {
    let (mut ctx, router) = make_test_context_with_config(SessionConfig {
        max_pending_requests: 100,
        max_subscriptions: 2,
        ..SessionConfig::default()
    });
    for i in 0..2 {
        ctx.register_subscription(uuid::Uuid::from_u128(i), &[[i as u8; 32]], &[]);
    }
    assert_eq!(ctx.subscription_count(), 2);

    use rekindle_transport_ipc::v3::handlers::channel::subscribe;
    let sub_id = uuid::Uuid::from_u128(99);
    let topic = [0x99; 32];
    let mut payload = Vec::new();
    payload.extend_from_slice(sub_id.as_bytes());
    payload.extend_from_slice(&1u32.to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());
    payload.extend_from_slice(&topic);
    let name = b"test";
    payload.extend_from_slice(&(name.len() as u16).to_le_bytes());
    payload.extend_from_slice(name);

    let result = subscribe::handle(&mut ctx, &payload);
    assert!(result.is_err(), "subscription beyond quota must be rejected");
    assert_no_router_deliveries(&router);
}
