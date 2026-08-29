//! Tests for [`super::EventDedup`] and [`super::hash_event`].

use super::*;
use rekindle_types::subscription_events::{FriendEvent, UnreadContext};

#[test]
fn same_event_produces_same_hash() {
    let event = SubscriptionEvent::Friend(FriendEvent::RequestReceived {
        from_key: "abc123".into(),
        display_name: "alice".into(),
        message: "hello".into(),
    });
    let h1 = hash_event(&event);
    let h2 = hash_event(&event);
    assert_eq!(h1, h2);
}

#[test]
fn different_events_produce_different_hashes() {
    let e1 = SubscriptionEvent::Friend(FriendEvent::RequestReceived {
        from_key: "abc123".into(),
        display_name: "alice".into(),
        message: "hello".into(),
    });
    let e2 = SubscriptionEvent::Friend(FriendEvent::RequestReceived {
        from_key: "def456".into(),
        display_name: "bob".into(),
        message: "hello".into(),
    });
    assert_ne!(hash_event(&e1), hash_event(&e2));
}

#[test]
fn dedup_suppresses_duplicate() {
    let mut dedup = EventDedup::new(100, 300);
    let event = SubscriptionEvent::Friend(FriendEvent::Accepted {
        peer_key: "abc123".into(),
        dm_log_key: "log1".into(),
    });
    assert!(dedup.check(&event)); // first: emit
    assert!(!dedup.check(&event)); // second: suppress
    assert_eq!(dedup.suppressed_count(), 1);
}

#[test]
fn unread_changed_never_deduped() {
    let mut dedup = EventDedup::new(100, 300);
    let event = SubscriptionEvent::UnreadChanged {
        context: UnreadContext::FriendRequests,
        count: 3,
    };
    assert!(dedup.check(&event));
    assert!(dedup.check(&event)); // same event, still emitted
    assert_eq!(dedup.suppressed_count(), 0);
}

#[test]
fn capacity_eviction() {
    let mut dedup = EventDedup::new(2, 300);
    let e1 = SubscriptionEvent::Friend(FriendEvent::Removed {
        peer_key: "a".into(),
    });
    let e2 = SubscriptionEvent::Friend(FriendEvent::Removed {
        peer_key: "b".into(),
    });
    let e3 = SubscriptionEvent::Friend(FriendEvent::Removed {
        peer_key: "c".into(),
    });

    assert!(dedup.check(&e1));
    assert!(dedup.check(&e2));
    assert!(dedup.check(&e3)); // evicts e1
    assert!(dedup.check(&e1)); // e1 was evicted, so it's new again
    assert_eq!(dedup.len(), 2);
}
