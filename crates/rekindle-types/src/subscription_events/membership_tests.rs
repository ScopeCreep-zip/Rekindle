use super::MembershipEvent;

/// A Joined whose card is not read yet carries a null name, never "".
#[test]
fn a_joined_before_its_card_carries_no_name() {
    let event = MembershipEvent::Joined {
        community: "c".into(),
        pseudonym: "p".into(),
        display_name: None,
        role_ids: Vec::new(),
    };
    let json = serde_json::to_value(&event).expect("serializes");
    assert!(json["joined"]["displayName"].is_null(), "{json}");
}

/// Our forget is device-scoped and names no community scope, so a
/// subscriber filtered to another community still receives it.
#[test]
fn a_forget_reaches_every_community_filter() {
    use crate::subscription_events::{EventScope, SubscriptionEvent, SubscriptionFilter};
    let event = SubscriptionEvent::Membership(MembershipEvent::Forgotten {
        community: "c".into(),
    });
    assert_eq!(event.scope(), EventScope::Device);
    assert_eq!(event.community(), None);
    let other = SubscriptionFilter {
        categories: Some(vec![crate::subscription_events::EventCategory::Membership]),
        community_scope: Some("other".into()),
    };
    assert!(
        other.matches(&event),
        "a filter on another community passes it"
    );
    let json = serde_json::to_value(&event).expect("serializes");
    assert_eq!(json["membership"]["forgotten"]["community"], "c", "{json}");
}

/// The removal's reason crosses the wire under `reason` and back.
#[test]
fn a_removal_reason_round_trips() {
    for banned in [false, true] {
        let event = if banned {
            MembershipEvent::Banned {
                community: "c".into(),
                target_pseudonym: "t".into(),
                removed_by: Some("b".into()),
                removed_by_name: Some("Ann".into()),
                reason: Some("spam".into()),
            }
        } else {
            MembershipEvent::Kicked {
                community: "c".into(),
                target_pseudonym: "t".into(),
                removed_by: None,
                removed_by_name: None,
                reason: Some("spam".into()),
            }
        };
        let json = serde_json::to_value(&event).expect("serializes");
        let tag = if banned { "banned" } else { "kicked" };
        assert_eq!(json[tag]["reason"], "spam", "{json}");
        let back: MembershipEvent = serde_json::from_value(json).expect("deserializes");
        let reason = match back {
            MembershipEvent::Kicked { reason, .. } | MembershipEvent::Banned { reason, .. } => {
                reason
            }
            _ => None,
        };
        assert_eq!(reason.as_deref(), Some("spam"));
    }
}
