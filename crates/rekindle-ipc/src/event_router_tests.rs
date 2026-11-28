use super::*;
use rekindle_types::subscription_events::{ChannelMessageEvent, FriendEvent};

fn make_tx() -> mpsc::Sender<Vec<u8>> {
    mpsc::channel(64).0
}

fn friend() -> SubscriptionEvent {
    SubscriptionEvent::Friend(FriendEvent::Removed {
        peer_key: "x".into(),
    })
}

fn channel_message(community: &str) -> SubscriptionEvent {
    SubscriptionEvent::ChannelMessage(ChannelMessageEvent::New {
        community: community.into(),
        channel: "gen".into(),
        message_id: "m1".into(),
        sender_pseudonym: "s".into(),
        sequence: 0,
        timestamp: 0,
        body: None,
        reply_to_message_id: None,
        forwarded_from_author: None,
        forwarded_from_name: None,
        automod_blurred: false,
        undecryptable: None,
    })
}

#[test]
fn wildcard_matches_every_event() {
    let mut router = EventRouter::new();
    router
        .subscribe(1, &[SubscriptionFilter::all()], make_tx())
        .unwrap();
    assert!(router.matches(1, &friend()));
    assert!(router.matches(1, &channel_message("gov1")));
    assert!(!router.matches(2, &friend()));
}

#[test]
fn category_filter_matches_its_category_only() {
    let mut router = EventRouter::new();
    router
        .subscribe(
            1,
            &[SubscriptionFilter::categories(vec![EventCategory::Friend])],
            make_tx(),
        )
        .unwrap();
    assert!(router.matches(1, &friend()));
    assert!(!router.matches(1, &channel_message("gov1")));
}

#[test]
fn community_scope_restricts_matching() {
    let mut router = EventRouter::new();
    router
        .subscribe(
            1,
            &[SubscriptionFilter::community("gov1".into())],
            make_tx(),
        )
        .unwrap();
    assert!(router.matches(1, &channel_message("gov1")));
    assert!(!router.matches(1, &channel_message("gov2")));
    // Global events pass community filters.
    assert!(router.matches(1, &friend()));
}

/// A community-scoped subscriber still receives device-wide events of
/// every category. Calls and notifications were missing from the
/// category list this expands, so such a subscriber never rang.
#[test]
fn community_scope_receives_calls_and_notifications() {
    use rekindle_types::subscription_events::{CallEvent, NotificationEvent};
    let mut router = EventRouter::new();
    router
        .subscribe(
            1,
            &[SubscriptionFilter::community("gov1".into())],
            make_tx(),
        )
        .unwrap();
    assert!(router.matches(
        1,
        &SubscriptionEvent::Call(CallEvent::Ringing {
            call_id: "c".into(),
        })
    ));
    assert!(router.matches(
        1,
        &SubscriptionEvent::Notification(NotificationEvent::SystemAlert {
            id: "a".into(),
            title: "t".into(),
            subject: None,
            body: "b".into(),
        })
    ));
}

#[test]
fn unsubscribe_removes_matching() {
    let mut router = EventRouter::new();
    let filter = SubscriptionFilter::all();
    router
        .subscribe(1, std::slice::from_ref(&filter), make_tx())
        .unwrap();
    assert!(router.matches(1, &friend()));
    router.unsubscribe(1, &[filter]);
    assert!(!router.matches(1, &friend()));
}

#[test]
fn remove_connection_cleans_everything() {
    let mut router = EventRouter::new();
    router
        .subscribe(1, &[SubscriptionFilter::all()], make_tx())
        .unwrap();
    assert_eq!(router.connection_count(), 1);
    router.remove_connection(1);
    assert_eq!(router.connection_count(), 0);
    assert_eq!(router.index_size(), 0);
}

#[test]
fn filter_limit_enforced() {
    let mut router = EventRouter::new();
    let filters: Vec<SubscriptionFilter> = (0..=MAX_FILTERS_PER_CONNECTION)
        .map(|_| SubscriptionFilter::all())
        .collect();
    assert!(router.subscribe(1, &filters, make_tx()).is_err());
}
