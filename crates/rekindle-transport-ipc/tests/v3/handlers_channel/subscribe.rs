use rekindle_transport_ipc::v3::context::OutboundFrame;
use rekindle_transport_ipc::v3::wire::frame_kind::ChannelKind;
use rekindle_transport_ipc::v3::dispatch::test_helpers::{make_test_context, assert_no_router_deliveries};
use rekindle_transport_ipc::v3::handlers::channel::subscribe;
use rekindle_transport_ipc::v3::handlers::channel::unsubscribe;

fn make_subscribe_payload(sub_id: uuid::Uuid, topics: &[[u8; 32]]) -> Vec<u8> {
    let mut buf = Vec::new();
    buf.extend_from_slice(sub_id.as_bytes());
    buf.extend_from_slice(&(topics.len() as u32).to_le_bytes());
    buf.extend_from_slice(&0u32.to_le_bytes());
    for topic in topics {
        buf.extend_from_slice(topic);
        let name = b"test-topic";
        buf.extend_from_slice(&(name.len() as u16).to_le_bytes());
        buf.extend_from_slice(name);
    }
    buf
}

#[test]
fn subscribe_adds_to_topic_index() {
    let (mut ctx, router) = make_test_context();
    let sub_id = uuid::Uuid::from_u128(100);
    let topic = [0x11; 32];
    subscribe::handle(&mut ctx, &make_subscribe_payload(sub_id, &[topic])).unwrap();
    assert_eq!(ctx.subscription_count(), 1);
    assert!(ctx.has_subscription_for_topic(&topic));
    assert_no_router_deliveries(&router);
}

#[test]
fn subscribe_produces_subscribe_ack() {
    let (mut ctx, router) = make_test_context();
    let sub_id = uuid::Uuid::from_u128(200);
    let topic = [0x22; 32];
    subscribe::handle(&mut ctx, &make_subscribe_payload(sub_id, &[topic])).unwrap();
    let out = ctx.drain_outbound();
    assert!(
        out.iter().any(|f| matches!(f, OutboundFrame::Channel { kind: ChannelKind::SubscribeAck, .. })),
        "subscribe must produce SUBSCRIBE_ACK"
    );
    assert_no_router_deliveries(&router);
}

#[test]
fn subscribe_multiple_topics() {
    let (mut ctx, router) = make_test_context();
    let sub_id = uuid::Uuid::from_u128(300);
    let topics = [[0x33; 32], [0x44; 32], [0x55; 32]];
    subscribe::handle(&mut ctx, &make_subscribe_payload(sub_id, &topics)).unwrap();
    assert_eq!(ctx.subscription_count(), 1);
    for topic in &topics {
        assert!(ctx.has_subscription_for_topic(topic));
    }
    assert_no_router_deliveries(&router);
}

#[test]
fn unsubscribe_removes_from_index() {
    let (mut ctx, router) = make_test_context();
    let sub_id = uuid::Uuid::from_u128(400);
    let topic = [0x66; 32];
    subscribe::handle(&mut ctx, &make_subscribe_payload(sub_id, &[topic])).unwrap();
    assert_eq!(ctx.subscription_count(), 1);

    let mut unsub_payload = Vec::new();
    unsub_payload.extend_from_slice(sub_id.as_bytes());
    unsub_payload.extend_from_slice(&0u32.to_le_bytes());
    unsub_payload.extend_from_slice(&0u32.to_le_bytes());
    unsubscribe::handle(&mut ctx, &unsub_payload).unwrap();
    assert_eq!(ctx.subscription_count(), 0);
    assert!(!ctx.has_subscription_for_topic(&topic));
    assert_no_router_deliveries(&router);
}

#[test]
fn unsubscribe_produces_unsubscribe_ack() {
    let (mut ctx, router) = make_test_context();
    let sub_id = uuid::Uuid::from_u128(500);
    let topic = [0x77; 32];
    subscribe::handle(&mut ctx, &make_subscribe_payload(sub_id, &[topic])).unwrap();
    ctx.drain_outbound();

    let mut unsub_payload = Vec::new();
    unsub_payload.extend_from_slice(sub_id.as_bytes());
    unsub_payload.extend_from_slice(&0u32.to_le_bytes());
    unsub_payload.extend_from_slice(&0u32.to_le_bytes());
    unsubscribe::handle(&mut ctx, &unsub_payload).unwrap();
    let out = ctx.drain_outbound();
    assert!(
        out.iter().any(|f| matches!(f, OutboundFrame::Channel { kind: ChannelKind::UnsubscribeAck, .. })),
        "unsubscribe must produce UNSUBSCRIBE_ACK"
    );
    assert_no_router_deliveries(&router);
}
