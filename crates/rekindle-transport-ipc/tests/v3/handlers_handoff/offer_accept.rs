use rekindle_transport_ipc::v3::context::OutboundFrame;
use rekindle_transport_ipc::v3::wire::frame_kind::HandoffKind;
use rekindle_transport_ipc::v3::dispatch::test_helpers::{make_test_context, assert_no_router_deliveries};
use rekindle_transport_ipc::v3::handlers::handoff::{offer, accept, reject};
use rekindle_transport_ipc::v3::codec::handoff::offer as offer_codec;

#[test]
fn offer_with_valid_mac_produces_accept() {
    let (mut ctx, router) = make_test_context();
    let handoff_id = uuid::Uuid::from_u128(100);
    let content_hash = [0xAB; 32];

    let offer_payload = offer_codec::HandoffOfferPayload {
        handoff_id, fd_kind: 0x01, fd_sealed: true, stream_id: 5,
        offer_timeout_ms: 5000, payload_size_bytes: 1024, content_hash,
    };

    let handoff_key = ctx.keys().handoff;
    let mut payload = offer_codec::encode(&offer_payload);
    let mac = blake3::keyed_hash(&handoff_key, &payload[..64]);
    payload[64..80].copy_from_slice(&mac.as_bytes()[..16]);

    offer::handle(&mut ctx, &payload).unwrap();

    let out = ctx.drain_outbound();
    assert!(
        out.iter().any(|f| matches!(f, OutboundFrame::Handoff { kind: HandoffKind::Accept, .. })),
        "offer with valid MAC must produce HANDOFF_ACCEPT"
    );
    assert_no_router_deliveries(&router);
}

#[test]
fn offer_with_invalid_mac_produces_reject() {
    let (mut ctx, router) = make_test_context();
    let offer_payload = offer_codec::HandoffOfferPayload {
        handoff_id: uuid::Uuid::from_u128(200), fd_kind: 0x01, fd_sealed: true,
        stream_id: 3, offer_timeout_ms: 5000, payload_size_bytes: 2048, content_hash: [0xCD; 32],
    };

    let mut payload = offer_codec::encode(&offer_payload);
    payload[64..80].copy_from_slice(&[0xFF; 16]);

    let result = offer::handle(&mut ctx, &payload);
    assert!(result.is_err());

    let out = ctx.drain_outbound();
    assert!(
        out.iter().any(|f| matches!(f, OutboundFrame::Handoff { kind: HandoffKind::Reject, .. })),
        "offer with invalid MAC must produce HANDOFF_REJECT"
    );
    assert_eq!(ctx.fallback_tracker().consecutive_failures(), 1);
    assert_no_router_deliveries(&router);
}

#[test]
fn accept_records_success_in_fallback() {
    let (mut ctx, router) = make_test_context();
    let handoff_id = uuid::Uuid::from_u128(42);
    ctx.register_pending_handoff(handoff_id);

    let mut payload = vec![0u8; 64];
    payload[0..16].copy_from_slice(handoff_id.as_bytes());
    payload[16..24].copy_from_slice(&0u64.to_le_bytes());
    payload[32..64].copy_from_slice(&[0xAA; 32]);

    accept::handle(&mut ctx, &payload).unwrap();
    assert_eq!(ctx.fallback_tracker().consecutive_failures(), 0);
    assert_no_router_deliveries(&router);
}

#[test]
fn reject_records_failure_in_fallback() {
    let (mut ctx, router) = make_test_context();
    let handoff_id = uuid::Uuid::from_u128(43);
    ctx.register_pending_handoff(handoff_id);

    let mut payload = Vec::new();
    payload.extend_from_slice(&1u32.to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());
    payload.extend_from_slice(handoff_id.as_bytes());

    reject::handle(&mut ctx, &payload).unwrap();
    assert_eq!(ctx.fallback_tracker().consecutive_failures(), 1);
    assert_no_router_deliveries(&router);
}
