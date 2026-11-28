//! Tests for [`super::EventDedup`] and [`super::hash_event`].

use super::*;
use rekindle_types::subscription_events::{
    CallEvent, CallOutcome, FocusReason, FriendEvent, TypingContext, TypingEvent, UnreadContext,
};

/// A fixed clock reading: dedup is a pure function of the events and `now`.
const NOW_MS: u64 = 1_700_000_000_000;

#[test]
fn call_dedup_uses_lifecycle_steps_and_distinguishes_focus_reasons() {
    let declined = |peer_reason| {
        SubscriptionEvent::Call(CallEvent::Declined {
            call_id: "c1".into(),
            notice: CallOutcome::Declined.notice(),
            peer_reason,
        })
    };
    let focus = |reason| {
        SubscriptionEvent::Call(CallEvent::FocusConversation {
            call_id: "c1".into(),
            peer_key: "pk".into(),
            peer_display_name: "Ada".into(),
            reason,
        })
    };
    let mut dedup = EventDedup::new(100, 300);
    assert!(dedup.check(&declined(None), NOW_MS));
    assert!(!dedup.check(&declined(Some("busy".into())), NOW_MS));
    assert!(dedup.check(&focus(FocusReason::CallStarted), NOW_MS));
    assert!(dedup.check(&focus(FocusReason::CallAccepted), NOW_MS));
    assert!(!dedup.check(&focus(FocusReason::CallStarted), NOW_MS));
}

#[test]
fn time_buckets_come_from_the_callers_clock() {
    let mut dedup = EventDedup::new(100, 300);
    let event = SubscriptionEvent::Typing(TypingEvent::Started {
        context: TypingContext::Dm {
            peer_key: "abc123".into(),
        },
        who: "alice".into(),
        name: "Alice".into(),
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

/// Stop, start, stop: the same `paused` state twice is two real
/// changes, so neither is suppressed (plan E7.2 G12).
#[test]
fn transfer_events_never_deduped() {
    use rekindle_types::dto::transfer::TransferState;
    use rekindle_types::subscription_events::TransferEvent;
    let mut dedup = EventDedup::new(100, 300);
    let event = SubscriptionEvent::Transfer(TransferEvent::StateChanged {
        attachment_id: "att".into(),
        state: TransferState::Paused,
    });
    assert!(dedup.check(&event, NOW_MS));
    assert!(dedup.check(&event, NOW_MS));
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

/// Two bio changes from one friend inside one bucket are two events: the
/// new bio's digest is part of the hash, so the second is not suppressed.
#[test]
fn two_bio_changes_in_one_bucket_are_both_kept() {
    let mut dedup = EventDedup::new(100, 300);
    let bio = |digest: &str| {
        SubscriptionEvent::Friend(FriendEvent::BioChanged {
            peer_key: "abc123".into(),
            bio_digest: digest.into(),
        })
    };
    assert!(dedup.check(&bio("d1"), NOW_MS));
    assert!(
        dedup.check(&bio("d2"), NOW_MS + 1),
        "a second change is kept"
    );
    assert!(
        !dedup.check(&bio("d2"), NOW_MS + 2),
        "the same change is not"
    );
}

/// SR2b: successive retries of one group send are distinct changes, not
/// one change repeated, so the dedup window never merges them.
#[test]
fn successive_group_send_retries_are_not_merged() {
    use rekindle_types::message::{DeliveryRetry, SendState};
    use rekindle_types::subscription_events::GroupDmEvent;
    let retry = |attempts| {
        SubscriptionEvent::GroupDm(GroupDmEvent::SendStatus {
            group_id: "g1".into(),
            client_id: "c1".into(),
            state: SendState::Sent,
            retry: Some(DeliveryRetry {
                attempts,
                next_attempt_at: 9_000,
            }),
        })
    };
    let mut dedup = EventDedup::new(100, 300);
    assert!(dedup.check(&retry(1), NOW_MS));
    assert!(
        dedup.check(&retry(2), NOW_MS),
        "the second retry is published"
    );
}
