use super::*;

#[test]
fn mock_node_default_is_detached() {
    let mock = MockNode::new();
    assert!(!mock.shared().is_attached());
    assert!(!mock.shared().public_internet_ready());
    assert_eq!(mock.shared().attachment_state(), AttachmentState::Detached);
}

#[test]
fn mock_node_attached_constructor() {
    let mock = MockNode::attached();
    assert!(mock.shared().is_attached());
    assert!(mock.shared().public_internet_ready());
    assert_eq!(
        mock.shared().attachment_state(),
        AttachmentState::FullyAttached
    );
}

#[test]
fn mock_node_add_peer() {
    let mock = MockNode::new();
    assert_eq!(mock.peers().read().route_count(), 0);

    mock.add_peer("peer1", vec![1, 2, 3]);
    assert_eq!(mock.peers().read().route_count(), 1);

    mock.add_peer("peer2", vec![4, 5, 6]);
    assert_eq!(mock.peers().read().route_count(), 2);
}

#[test]
fn mock_node_circuit_breaker() {
    let mock = MockNode::new();
    mock.add_peer("peer1", vec![1, 2, 3]);

    assert!(!mock.peers().read().is_circuit_open("peer1"));

    mock.trip_circuit("peer1");
    assert!(mock.peers().read().is_circuit_open("peer1"));

    let summary = mock.circuit_summary();
    assert_eq!(summary.circuit_open, 1);
}

#[test]
fn mock_node_mek_cache() {
    use rekindle_types::channel_keys::KeyScope;
    use rekindle_types::id::ChannelId;
    let chan1 = KeyScope::Channel(ChannelId([1; 16]));
    let chan2 = KeyScope::Channel(ChannelId([2; 16]));
    let mock = MockNode::new();
    mock.add_mek("comm1", chan1, 1);
    mock.add_mek("comm1", chan1, 2);
    mock.add_mek("comm1", chan2, 1);

    let snapshot = mock.mek_snapshot("comm1");
    assert_eq!(snapshot.len(), 3);

    let cache = mock.mek_cache().read();
    assert_eq!(cache.current("comm1", chan1).unwrap().generation(), 2);
    assert_eq!(cache.current("comm1", chan2).unwrap().generation(), 1);
}

#[tokio::test]
async fn mock_node_subscribe_receives_injected_events() {
    let mock = MockNode::new();
    let mut rx = mock.subscribe();

    mock.inject(&TransportNotification::DmReceived {
        sender_key: "abc123".into(),
        sender_name: "alice".into(),
        timestamp: 1234,
    });

    let event = rx.try_recv().expect("should receive event");
    match event {
        TransportNotification::DmReceived { sender_key, .. } => {
            assert_eq!(sender_key, "abc123");
        }
        other => panic!("unexpected: {other:?}"),
    }
}

#[test]
fn mock_node_status_snapshot() {
    let mock = MockNode::attached();
    mock.add_peer("p1", vec![1]);
    mock.add_peer("p2", vec![2]);

    let status = mock.status_snapshot();
    assert!(status.is_attached);
    assert!(status.public_internet_ready);
    assert_eq!(status.peer_count, 2);
    assert!(!status.route_allocated);

    // After setting the override, route_allocated should be true
    mock.set_route_allocated(true);
    let status2 = mock.status_snapshot();
    assert!(status2.route_allocated);
}

#[test]
fn mock_node_peer_snapshot_sorted() {
    let mock = MockNode::new();
    mock.add_peer("zzz_peer", vec![1]);
    mock.add_peer("aaa_peer", vec![2]);

    let snapshot = mock.peer_snapshot();
    assert_eq!(snapshot.len(), 2);
    assert_eq!(snapshot[0].key, "aaa_peer");
    assert_eq!(snapshot[1].key, "zzz_peer");
}

#[test]
fn mock_node_set_specific_attachment() {
    let mock = MockNode::new();
    mock.set_attachment_state(AttachmentState::AttachedWeak, true, false);

    assert!(mock.shared().is_attached());
    assert!(!mock.shared().public_internet_ready());
    assert_eq!(
        mock.shared().attachment_state(),
        AttachmentState::AttachedWeak
    );
}

#[test]
fn mock_node_empty_route_blob_ignored() {
    let mock = MockNode::new();
    mock.add_peer("peer1", Vec::new()); // empty blob
    assert_eq!(mock.peers().read().route_count(), 0); // should not be cached
}
