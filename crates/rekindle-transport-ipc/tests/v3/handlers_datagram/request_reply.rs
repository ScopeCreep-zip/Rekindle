use rekindle_transport_ipc::v3::context::SessionConfig;
use rekindle_transport_ipc::v3::dispatch::test_helpers::{make_test_context, make_test_context_with_config};
use rekindle_transport_ipc::v3::handlers::datagram::{request, reply};
use rekindle_transport_ipc::v3::codec::datagram::{request as req_codec, reply as reply_codec};
use rekindle_transport_ipc::v3::wire::clearance::Clearance;

#[test]
fn request_registers_pending() {
    let (mut ctx, router) = make_test_context();
    let msg_id = uuid::Uuid::from_u128(42);
    let payload = req_codec::encode(&req_codec::DatagramRequestPayload {
        message_id: msg_id,
        reply_timeout_ms: 5000,
        sender_clearance: Clearance::Internal,
        application_payload: vec![1, 2, 3],
        conditions: vec![],
    });
    request::handle(&mut ctx, &payload).unwrap();
    assert_eq!(ctx.pending_request_count(), 1);

    // Verify the request was delivered to the router with correct data
    let requests = router.requests.lock();
    assert_eq!(requests.len(), 1, "request handler must deliver to router");
    assert_eq!(requests[0].message_id, msg_id);
    assert_eq!(requests[0].sender_clearance, Clearance::Internal);
    assert_eq!(requests[0].payload, vec![1, 2, 3]);
}

#[test]
fn reply_resolves_pending() {
    let (mut ctx, router) = make_test_context();
    let msg_id = uuid::Uuid::from_u128(42);
    ctx.register_pending_request(msg_id, 5000);
    assert_eq!(ctx.pending_request_count(), 1);

    let reply_msg_id = uuid::Uuid::from_u128(100);
    let payload = reply_codec::encode(&reply_codec::DatagramReplyPayload {
        message_id: reply_msg_id,
        correlation_id: msg_id,
        status_phase: 0x03, // Succeeded
        application_payload: vec![4, 5, 6],
        status_reason: "ok".into(),
        status_message: String::new(),
        conditions: vec![],
    });
    reply::handle(&mut ctx, &payload).unwrap();
    assert_eq!(ctx.pending_request_count(), 0);

    // Verify the reply was delivered to the router with correct data
    let replies = router.replies.lock();
    assert_eq!(replies.len(), 1, "reply handler must deliver to router");
    assert_eq!(replies[0].message_id, reply_msg_id);
    assert_eq!(replies[0].correlation_id, msg_id);
    assert_eq!(replies[0].status_phase, 0x03);
    assert_eq!(replies[0].payload, vec![4, 5, 6]);
}

#[test]
fn reply_for_unknown_correlation_is_orphan() {
    let (mut ctx, router) = make_test_context();
    let payload = reply_codec::encode(&reply_codec::DatagramReplyPayload {
        message_id: uuid::Uuid::from_u128(200),
        correlation_id: uuid::Uuid::from_u128(999),
        status_phase: 0x03,
        application_payload: vec![],
        status_reason: String::new(),
        status_message: String::new(),
        conditions: vec![],
    });
    let result = reply::handle(&mut ctx, &payload);
    assert!(result.is_ok()); // orphan is logged, not an error

    // Even orphan replies are delivered to the router — the application decides
    let replies = router.replies.lock();
    assert_eq!(replies.len(), 1, "orphan reply must still be delivered to router");
    assert_eq!(replies[0].correlation_id, uuid::Uuid::from_u128(999));
}

#[test]
fn pending_requests_overflow_rejected() {
    let (mut ctx, router) = make_test_context_with_config(SessionConfig {
        max_pending_requests: 2,
        ..SessionConfig::default()
    });

    for i in 0..2 {
        ctx.register_pending_request(uuid::Uuid::from_u128(i), 5000);
    }
    assert_eq!(ctx.pending_request_count(), 2);

    let payload = req_codec::encode(&req_codec::DatagramRequestPayload {
        message_id: uuid::Uuid::from_u128(999),
        reply_timeout_ms: 5000,
        sender_clearance: Clearance::Internal,
        application_payload: vec![],
        conditions: vec![],
    });
    let result = request::handle(&mut ctx, &payload);
    assert!(result.is_err());

    // Overflow request must NOT be delivered to router
    let requests = router.requests.lock();
    assert_eq!(requests.len(), 0, "rejected overflow request must not reach router");
}
