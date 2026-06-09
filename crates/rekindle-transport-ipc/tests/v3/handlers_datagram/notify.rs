use rekindle_transport_ipc::v3::context::OutboundFrame;
use rekindle_transport_ipc::v3::dispatch::test_helpers::make_test_context;
use rekindle_transport_ipc::v3::handlers::datagram::notify;
use rekindle_transport_ipc::v3::codec::datagram::notify as notify_codec;
use rekindle_transport_ipc::v3::codec::channel::ack as ack_codec;
use rekindle_transport_ipc::v3::wire::clearance::Clearance;
use rekindle_transport_ipc::v3::wire::frame_kind::ChannelKind;

#[test]
fn notify_produces_ack_referencing_correct_message_id() {
    let (mut ctx, router) = make_test_context();
    let msg_id = uuid::Uuid::from_u128(42);
    let payload = notify_codec::encode(&notify_codec::DatagramNotifyPayload {
        message_id: msg_id,
        sender_clearance: Clearance::Internal,
        application_payload: vec![1, 2, 3],
    });
    notify::handle(&mut ctx, &payload).unwrap();

    let notifications = router.notifications.lock();
    assert_eq!(notifications.len(), 1, "notify handler must deliver to router");
    assert_eq!(notifications[0].message_id, msg_id);
    assert_eq!(notifications[0].sender_clearance, Clearance::Internal);
    assert_eq!(notifications[0].payload, vec![1, 2, 3]);
    drop(notifications);

    let out = ctx.drain_outbound();
    let ack_frame = out.iter()
        .find(|f| matches!(f, OutboundFrame::Channel { kind: ChannelKind::Ack, .. }))
        .expect("notify must produce CHANNEL_ACK");

    match ack_frame {
        OutboundFrame::Channel { payload, .. } => {
            let ack = ack_codec::decode(payload).expect("ACK payload must decode");
            assert!(
                ack.message_ids.contains(&msg_id),
                "ACK must reference the notify's message_id {msg_id}, got {:?}",
                ack.message_ids,
            );
        }
        _ => unreachable!(),
    }
}
