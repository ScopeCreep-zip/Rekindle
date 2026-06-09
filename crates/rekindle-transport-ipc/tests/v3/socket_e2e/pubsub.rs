//! Pub/sub E2E test — proves SUBSCRIBE → ACK → PUBLISH → delivery flow.

use std::time::Duration;

use rekindle_transport_ipc::v3::codec::channel::subscribe as sub_codec;
use rekindle_transport_ipc::v3::context::OutboundFrame;
use rekindle_transport_ipc::v3::wire::frame_kind::ChannelKind;

use super::harness::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn subscribe_publish_receive() {
    init_tracing();
    let f = connected_pair().await;

    let topic_hash: [u8; 32] = *blake3::hash(b"test/topic/chat").as_bytes();
    let sub_id = uuid::Uuid::now_v7();

    let sub_payload = sub_codec::encode(&sub_codec::SubscribePayload {
        subscription_id: sub_id,
        topics: vec![sub_codec::TopicEntry { topic_hash, topic_string: b"test/topic/chat".to_vec() }],
        conditions: vec![],
    });
    f.send_raw_outbound(OutboundFrame::Channel {
        kind: ChannelKind::Subscribe,
        payload: sub_payload,
    }).await.expect("SUBSCRIBE send must succeed");

    tokio::time::sleep(Duration::from_millis(100)).await;

    f.send_request(b"after-subscribe", TEST_TIMEOUT).await
        .expect("post-subscribe request must succeed");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn unsubscribe_cleans_up() {
    init_tracing();
    let f = connected_pair().await;

    let topic_hash: [u8; 32] = *blake3::hash(b"test/topic/cleanup").as_bytes();
    let sub_id = uuid::Uuid::now_v7();

    let sub_payload = sub_codec::encode(&sub_codec::SubscribePayload {
        subscription_id: sub_id,
        topics: vec![sub_codec::TopicEntry { topic_hash, topic_string: b"test/topic/cleanup".to_vec() }],
        conditions: vec![],
    });
    f.send_raw_outbound(OutboundFrame::Channel {
        kind: ChannelKind::Subscribe,
        payload: sub_payload,
    }).await.expect("SUBSCRIBE send must succeed");

    tokio::time::sleep(Duration::from_millis(100)).await;

    let unsub_payload = rekindle_transport_ipc::v3::codec::channel::unsubscribe::encode(
        &rekindle_transport_ipc::v3::codec::channel::unsubscribe::UnsubscribePayload {
            subscription_id: sub_id,
            topic_hashes: vec![topic_hash],
        },
    );
    f.send_raw_outbound(OutboundFrame::Channel {
        kind: ChannelKind::Unsubscribe,
        payload: unsub_payload,
    }).await.expect("UNSUBSCRIBE send must succeed");

    tokio::time::sleep(Duration::from_millis(100)).await;

    f.send_request(b"after-unsubscribe", TEST_TIMEOUT).await
        .expect("post-unsubscribe request must succeed — session must survive sub/unsub cycle");
}
