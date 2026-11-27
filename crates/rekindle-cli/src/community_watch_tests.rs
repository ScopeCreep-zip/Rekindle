use super::render;
use rekindle_types::subscription_events::{MembershipEvent, SubscriptionEvent};

fn kicked(reason: Option<&str>) -> SubscriptionEvent {
    SubscriptionEvent::Membership(MembershipEvent::Kicked {
        community: "c".repeat(64),
        target_pseudonym: "t".repeat(64),
        removed_by: Some("b".repeat(64)),
        removed_by_name: Some("Ann".into()),
        reason: reason.map(str::to_owned),
    })
}

/// A kick shows who removed the member and why, in text and JSONL.
#[test]
fn a_kick_streams_its_remover_and_reason() {
    let event = kicked(Some("spam\u{1b}[2J"));
    let line = render(&event).expect("a kick is a community event");
    assert!(line.ends_with("was kicked by Ann: spam"), "{line}");
    let json = crate::output::format::jsonl_line(&event).expect("serializes");
    let value: serde_json::Value = serde_json::from_str(&json).expect("one JSON object");
    assert_eq!(value["membership"]["kicked"]["reason"], "spam\u{1b}[2J");
    assert!(!json.contains('\n'), "one line per event: {json}");
    let bare = render(&kicked(None)).expect("a kick is a community event");
    assert!(bare.ends_with("was kicked by Ann"), "{bare}");
}

/// Our leave streams a line in text and its camelCase tag in JSONL.
#[test]
fn our_forget_streams_a_line() {
    let event = SubscriptionEvent::Membership(MembershipEvent::Forgotten {
        community: "c".repeat(64),
    });
    let line = render(&event).expect("a forget is a community event");
    assert!(line.ends_with(": we no longer belong to it"), "{line}");
    let json = crate::output::format::jsonl_line(&event).expect("serializes");
    let value: serde_json::Value = serde_json::from_str(&json).expect("one JSON object");
    assert_eq!(
        value["membership"]["forgotten"]["community"],
        "c".repeat(64)
    );
}

/// A ban without a remover name falls back to the remover's key.
#[test]
fn a_ban_names_the_remover_key_without_a_card() {
    let event = SubscriptionEvent::Membership(MembershipEvent::Banned {
        community: "c".repeat(64),
        target_pseudonym: "t".repeat(64),
        removed_by: Some("b".repeat(64)),
        removed_by_name: None,
        reason: None,
    });
    let line = render(&event).expect("a ban is a community event");
    assert!(line.ends_with("was banned by bbbbbbbb…bbbb"), "{line}");
}

/// A join before the card is read shows the key alone; the card's name
/// follows as its own line (plan E3.2 F9-2).
#[test]
fn a_join_shows_the_key_until_the_card_names_the_member() {
    let joined = SubscriptionEvent::Membership(MembershipEvent::Joined {
        community: "c".repeat(64),
        pseudonym: "p".repeat(64),
        display_name: None,
        role_ids: Vec::new(),
    });
    let line = render(&joined).expect("a join is a community event");
    assert!(line.ends_with(": pppppppp…pppp joined"), "{line}");
    let card = SubscriptionEvent::Membership(MembershipEvent::MemberCardChanged {
        community: "c".repeat(64),
        pseudonym: "p".repeat(64),
        display_name: "Ann".into(),
        custom_status: None,
    });
    let line = render(&card).expect("a card name is a community event");
    assert!(
        line.ends_with(": Ann (pppppppp…pppp) is named on its card"),
        "{line}"
    );
}

/// Events outside a community are not this watch's.
#[test]
fn other_events_are_not_shown() {
    let event = SubscriptionEvent::UnreadChanged {
        context: rekindle_types::subscription_events::UnreadContext::FriendRequests,
        count: 1,
    };
    assert!(render(&event).is_none());
}
