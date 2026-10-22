//! Tests for [`super::EventDedup`] and [`super::hash_event`].

use super::*;
use rekindle_types::subscription_events::{FriendEvent, TypingContext, TypingEvent, UnreadContext};

/// A fixed clock reading: dedup is a pure function of the events and `now`.
const NOW_MS: u64 = 1_700_000_000_000;

#[test]
fn time_buckets_come_from_the_callers_clock() {
    let mut dedup = EventDedup::new(100, 300);
    let event = SubscriptionEvent::Typing(TypingEvent::Started {
        context: TypingContext::Dm {
            peer_key: "abc123".into(),
        },
        who: "alice".into(),
    });
    assert!(dedup.check(&event, NOW_MS));
    assert!(!dedup.check(&event, NOW_MS + 1), "same 5 s bucket");
    assert!(dedup.check(&event, NOW_MS + 5_000), "next bucket");
}

#[test]
fn same_event_produces_same_hash() {
    let event = SubscriptionEvent::Friend(FriendEvent::RequestReceived {
        from_key: "abc123".into(),
        display_name: "alice".into(),
        message: "hello".into(),
    });
    let h1 = hash_event(&event, NOW_MS);
    let h2 = hash_event(&event, NOW_MS);
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
    assert_ne!(hash_event(&e1, NOW_MS), hash_event(&e2, NOW_MS));
}

#[test]
fn dedup_suppresses_duplicate() {
    let mut dedup = EventDedup::new(100, 300);
    let event = SubscriptionEvent::Friend(FriendEvent::Accepted {
        peer_key: "abc123".into(),
        display_name: None,
    });
    assert!(dedup.check(&event, NOW_MS)); // first: emit
    assert!(!dedup.check(&event, NOW_MS)); // second: suppress
    assert_eq!(dedup.suppressed_count(), 1);
}

#[test]
fn unread_changed_never_deduped() {
    let mut dedup = EventDedup::new(100, 300);
    let event = SubscriptionEvent::UnreadChanged {
        context: UnreadContext::FriendRequests,
        count: 3,
    };
    assert!(dedup.check(&event, NOW_MS));
    assert!(dedup.check(&event, NOW_MS)); // same event, still emitted
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

    assert!(dedup.check(&e1, NOW_MS));
    assert!(dedup.check(&e2, NOW_MS));
    assert!(dedup.check(&e3, NOW_MS)); // evicts e1
    assert!(dedup.check(&e1, NOW_MS)); // e1 was evicted, so it's new again
    assert_eq!(dedup.len(), 2);
}
