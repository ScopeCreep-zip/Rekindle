use rekindle_transport_ipc::v3::dispatch::test_helpers::{make_test_context, assert_no_router_deliveries};
use rekindle_transport_ipc::v3::handlers::channel::ack;
use rekindle_transport_ipc::v3::handlers::channel::nack;
use rekindle_transport_ipc::v3::codec::channel::nack as nack_codec;
use rekindle_transport_ipc::v3::wire::failure::FailureCode;

fn encode_ack(message_ids: &[uuid::Uuid]) -> Vec<u8> {
    let mut buf = Vec::new();
    buf.extend_from_slice(&(message_ids.len() as u16).to_le_bytes());
    buf.extend_from_slice(&[0; 6]);
    buf.extend_from_slice(&0u64.to_le_bytes());
    for id in message_ids {
        buf.extend_from_slice(id.as_bytes());
    }
    buf
}

#[test]
fn ack_resolves_pending_request() {
    let (mut ctx, router) = make_test_context();
    let msg_id = uuid::Uuid::from_u128(42);
    ctx.register_pending_request(msg_id, 5000);
    assert_eq!(ctx.pending_request_count(), 1);
    ack::handle(&mut ctx, &encode_ack(&[msg_id])).unwrap();
    assert_eq!(ctx.pending_request_count(), 0);
    assert_no_router_deliveries(&router);
}

#[test]
fn ack_for_unknown_message_id_is_noop() {
    let (mut ctx, router) = make_test_context();
    let unknown = uuid::Uuid::from_u128(999);
    let result = ack::handle(&mut ctx, &encode_ack(&[unknown]));
    assert!(result.is_ok());
    assert_no_router_deliveries(&router);
}

#[test]
fn ack_batch_resolves_multiple() {
    let (mut ctx, router) = make_test_context();
    let ids: Vec<uuid::Uuid> = (0..5).map(|i| uuid::Uuid::from_u128(i)).collect();
    for &id in &ids {
        ctx.register_pending_request(id, 5000);
    }
    assert_eq!(ctx.pending_request_count(), 5);
    ack::handle(&mut ctx, &encode_ack(&ids)).unwrap();
    assert_eq!(ctx.pending_request_count(), 0);
    assert_no_router_deliveries(&router);
}

#[test]
fn nack_logs_rejection() {
    let (mut ctx, router) = make_test_context();
    let msg_id = uuid::Uuid::from_u128(77);
    ctx.register_pending_request(msg_id, 5000);

    let payload = nack_codec::encode(&nack_codec::NackPayload {
        reason_code: FailureCode::ClearanceInsufficient,
        rejected_message_id: msg_id,
        session_seq_rejected: 10,
        detail: "not enough clearance".into(),
    });
    nack::handle(&mut ctx, &payload).unwrap();
    assert_eq!(ctx.pending_request_count(), 0);
    assert_no_router_deliveries(&router);
}
