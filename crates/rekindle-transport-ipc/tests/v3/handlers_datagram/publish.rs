use rekindle_transport_ipc::v3::dispatch::test_helpers::make_test_context;
use rekindle_transport_ipc::v3::handlers::datagram::publish;
use rekindle_transport_ipc::v3::handlers::channel::subscribe;
use rekindle_transport_ipc::v3::codec::datagram::publish as pub_codec;
use rekindle_transport_ipc::v3::wire::clearance::Clearance;

#[test]
fn publish_routes_to_matching_subscription() {
    let (mut ctx, router) = make_test_context();
    let topic = [0x11; 32];
    let sub_id = uuid::Uuid::from_u128(1);

    // Register subscription via the subscribe handler
    let mut sub_payload = Vec::new();
    sub_payload.extend_from_slice(sub_id.as_bytes());
    sub_payload.extend_from_slice(&1u32.to_le_bytes()); // topic_count
    sub_payload.extend_from_slice(&0u32.to_le_bytes()); // conditions_len
    sub_payload.extend_from_slice(&topic);
    let name = b"test-topic";
    sub_payload.extend_from_slice(&(name.len() as u16).to_le_bytes());
    sub_payload.extend_from_slice(name);
    subscribe::handle(&mut ctx, &sub_payload).unwrap();
    ctx.drain_outbound(); // consume SUBSCRIBE_ACK

    let payload = pub_codec::encode(&pub_codec::DatagramPublishPayload {
        message_id: uuid::Uuid::from_u128(10),
        topic_hash: topic,
        event_seq: 1,
        event_timestamp_ns: 0,
        sender_clearance: Clearance::Internal,
        application_payload: vec![0xDE, 0xAD],
        conditions: vec![],
    });
    publish::handle(&mut ctx, &payload).unwrap();

    let publishes = router.publishes.lock();
    assert_eq!(publishes.len(), 1, "publish must route to exactly 1 matching subscription");
    assert_eq!(publishes[0].subscription_id, sub_id);
    assert_eq!(publishes[0].topic_hash, topic);
    assert_eq!(publishes[0].event_seq, 1);
    assert_eq!(publishes[0].payload, vec![0xDE, 0xAD]);
}

#[test]
fn publish_skips_unmatched_topic() {
    let (mut ctx, router) = make_test_context();
    let subscribed_topic = [0x11; 32];
    let other_topic = [0x22; 32];
    ctx.register_subscription(uuid::Uuid::from_u128(1), &[subscribed_topic], &[]);

    let payload = pub_codec::encode(&pub_codec::DatagramPublishPayload {
        message_id: uuid::Uuid::from_u128(10),
        topic_hash: other_topic,
        event_seq: 1,
        event_timestamp_ns: 0,
        sender_clearance: Clearance::Internal,
        application_payload: vec![],
        conditions: vec![],
    });
    publish::handle(&mut ctx, &payload).unwrap();

    let publishes = router.publishes.lock();
    assert_eq!(publishes.len(), 0, "publish to unmatched topic must not route to any subscription");
}

#[test]
fn publish_without_subscriptions_is_noop() {
    let (mut ctx, router) = make_test_context();
    let payload = pub_codec::encode(&pub_codec::DatagramPublishPayload {
        message_id: uuid::Uuid::from_u128(10),
        topic_hash: [0xFF; 32],
        event_seq: 1,
        event_timestamp_ns: 0,
        sender_clearance: Clearance::Internal,
        application_payload: vec![],
        conditions: vec![],
    });
    let result = publish::handle(&mut ctx, &payload);
    assert!(result.is_ok());

    let publishes = router.publishes.lock();
    assert_eq!(publishes.len(), 0, "publish with no subscriptions must not deliver anything");
}
