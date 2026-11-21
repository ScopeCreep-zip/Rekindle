//! `event_dispatch`'s audience and channel tests.

use super::*;
use rekindle_types::subscription_events::{
    CallEvent, MembershipEvent, NotificationEvent, PresenceEvent, PresenceSnapshot, VoiceEvent,
    VoiceScope,
};

const PK: &str = "3f1a9c0b7e2d4f6a8b1c3e5d7f9a0b2c4d6e8f0a1b3c5d7e9f1a2b4c6d8e0f1a";
const CALL: &str = "0123456789abcdef0123456789abcdef";
const REC: &str = "VLD0:um7m8HxBluv6XceSaB3dK9Lq0ZpWtYv1NcRe4Gh7Jf2";
const GROUP: &str = "3f1a9c0b7e2d4f6a8b1c3e5d7f9a0b2c";

fn sub(e: SubscriptionEvent) -> Audience {
    WebviewEvent::Subscription(e).audience()
}

#[test]
fn notifications_belong_to_the_buddy_list_only() {
    let a = sub(SubscriptionEvent::Notification(
        NotificationEvent::SystemAlert {
            title: "t".into(),
            subject: None,
            body: "b".into(),
        },
    ));
    assert!(a.includes("buddy-list"));
    assert!(!a.includes("chat-3f1a9c0b7e2d4f6a"));
    assert!(!a.includes("settings"));
}

#[test]
fn a_direct_message_reaches_its_chat_window_only() {
    let a = sub(SubscriptionEvent::ChannelMessage(
        ChannelMessageEvent::DmSendStatus {
            client_id: "01900000-0000-7000-8000-000000000002".into(),
            conversation_id: PK.into(),
            state: rekindle_types::message::SendState::Sent,
        },
    ));
    assert!(a.includes("buddy-list"));
    assert!(a.includes("chat-3f1a9c0b7e2d4f6a"));
    assert!(!a.includes("chat-0000000000000000"));
    assert!(!a.includes("community-browser"));
    let gap = sub(SubscriptionEvent::ChannelMessage(
        ChannelMessageEvent::DmHistoryGap {
            peer_key: PK.into(),
            through_seq: 7,
        },
    ));
    assert!(gap.includes("chat-3f1a9c0b7e2d4f6a"));
    assert!(!gap.includes("chat-0000000000000000"));
}

#[test]
fn group_dms_reach_their_group_chat_window() {
    let a = sub(SubscriptionEvent::ChannelMessage(
        ChannelMessageEvent::DirectMessageReceived {
            peer_key: PK.into(),
            timestamp: 1,
            sender_name: None,
            body: None,
            decryption_failed: false,
            automod_blurred: false,
            conversation_id: GROUP.into(),
            server_message_id: None,
            reply_to_id: None,
            is_own: false,
        },
    ));
    assert!(a.includes(&labels::conversation_label(GROUP).unwrap()));
    assert!(!a.includes("chat-3f1a9c0b7e2d4f6a"));
}

#[test]
fn joins_reach_every_community_window() {
    let a = sub(SubscriptionEvent::Membership(
        MembershipEvent::JoinProgress {
            community: REC.into(),
            stage: String::new(),
            status: String::new(),
        },
    ));
    assert!(a.includes("community-browser"));
    assert!(a.includes("buddy-list"));
    assert!(!a.includes("settings"));
}

/// Plan E3.2 S7c.5: a quarantined governance op is a community event, so
/// it reaches every community window and the buddy list's community list,
/// in the golden sample's shape (`crates/rekindle-types/tests/golden`).
#[test]
fn governance_op_quarantined_audience_golden() {
    let event = WebviewEvent::Subscription(SubscriptionEvent::Governance(
        rekindle_types::subscription_events::GovernanceEvent::GovernanceOpQuarantined {
            community: REC.into(),
            op: "8".repeat(64),
            author: PK.into(),
            reason: rekindle_types::subscription_events::OpQuarantineReason::CheckpointEquivocation,
            cause: Some("9".repeat(64)),
        },
    ));
    let a = event.audience();
    assert!(a.includes("community-browser"));
    assert!(a.includes("buddy-list"));
    assert!(!a.includes("settings"));
    assert!(!a.includes("chat-3f1a9c0b7e2d4f6a"));
    let envelope = event.envelope(None).expect("serializes");
    assert_eq!(
        envelope.payload,
        serde_json::json!({ "governance": { "governanceOpQuarantined": {
            "community": REC,
            "op": "8".repeat(64),
            "author": PK,
            "reason": "checkpointEquivocation",
            "cause": "9".repeat(64),
        }}})
    );
}

#[test]
fn calls_reach_their_window_and_conversation_windows() {
    let a = sub(SubscriptionEvent::Call(CallEvent::Ringing {
        call_id: CALL.into(),
    }));
    assert!(a.includes("buddy-list"));
    assert!(a.includes("call-0123456789ab"));
    assert!(!a.includes("call-ffffffffffff"));
    assert!(a.includes("chat-3f1a9c0b7e2d4f6a"));
    assert!(!a.includes("community-browser"));
    let voice = sub(SubscriptionEvent::Voice(VoiceEvent::LocalJoined {
        scope: VoiceScope::Dm {
            peer_key: PK.into(),
        },
    }));
    assert!(voice.includes("call-0123456789ab"));
}

#[test]
fn own_records_stage_reaches_every_post_login_window_on_network_status() {
    use rekindle_types::subscription_events::{NetworkEvent, OwnRecordsPhase};
    let e = SubscriptionEvent::Network(NetworkEvent::OwnRecordsChanged {
        phase: OwnRecordsPhase::Failed,
        error: Some("no route".into()),
    });
    let a = sub(e.clone());
    // The gated surfaces: the buddy list's add-friend tabs and intent
    // dialog, and the chat windows' composers.
    assert!(a.includes("buddy-list"));
    assert!(a.includes(&labels::peer_labels(PK)[0]));
    assert!(a.includes(&labels::conversation_label(GROUP).unwrap()));
    assert!(a.includes("settings"));
    assert!(!a.includes("login"));
    assert_eq!(WebviewEvent::Subscription(e).channel(), "network-status");
}

#[test]
fn device_events_reach_every_post_login_window() {
    let a = sub(SubscriptionEvent::Presence(PresenceEvent::SelfChanged {
        public_key: PK.into(),
        snapshot: PresenceSnapshot::default(),
    }));
    assert!(a.includes("settings"));
    assert!(!a.includes("login"));
    // Our profile too, on the presence channel (it replaced the
    // shell's `profile-updated`).
    let profile = SubscriptionEvent::Presence(PresenceEvent::SelfProfileChanged {
        public_key: PK.into(),
        display_name: None,
        avatar_changed: true,
    });
    assert!(sub(profile.clone()).includes("settings"));
    assert!(!sub(profile.clone()).includes("login"));
    assert_eq!(
        WebviewEvent::Subscription(profile).channel(),
        "presence-event"
    );
}

#[test]
fn a_skin_change_reaches_every_window_kind_on_its_own_channel() {
    let e = WebviewEvent::Subscription(SubscriptionEvent::System(SystemEvent::SkinChanged {
        skin: rekindle_types::dto::skin_library::SkinApplied {
            id: "builtin:symbiosis".into(),
            base: rekindle_types::dto::skin::SkinBase::Dark,
            tokens_href: "skin://localhost/a/tokens.css".into(),
            regions: Vec::new(),
            palette: Vec::new(),
        },
    }));
    assert_eq!(e.channel(), "skin-event");
    let audience = e.audience();
    for label in [
        "login",
        "buddy-list",
        "settings",
        "chat-0123456789ab",
        "community-x",
        "profile-x",
        "call-0123456789ab",
    ] {
        assert!(audience.includes(label), "{label}");
    }
}

#[test]
fn a_skin_trial_reaches_the_settings_window_only() {
    let e = WebviewEvent::Subscription(SubscriptionEvent::System(SystemEvent::SkinTrialChanged {
        trial: None,
    }));
    assert_eq!(e.channel(), "skin-trial-event");
    let audience = e.audience();
    assert!(audience.includes("settings"));
    for label in ["login", "buddy-list", "chat-0123456789ab", "community-x"] {
        assert!(!audience.includes(label), "{label}");
    }
}

#[test]
fn system_events_ride_the_community_channel() {
    let e = WebviewEvent::Subscription(SubscriptionEvent::System(
        rekindle_types::subscription_events::SystemEvent::RaidDetected {
            community: REC.into(),
            joins_in_window: 25,
            max_joins_per_interval: 10,
            join_interval_seconds: 60,
        },
    ));
    assert_eq!(e.channel(), "community-event");
}

/// Finding IB2: a post the Host stored (`ChannelMessageEvent::New`, from the
/// outbox read) reaches the community window on `community-event`, in the
/// shape its `new` reducer reads (`dispatcher_messages.ts` `applyNew`).
#[test]
fn a_stored_post_reaches_the_community_window_as_new() {
    let e = WebviewEvent::Subscription(SubscriptionEvent::ChannelMessage(
        rekindle_types::subscription_events::ChannelMessageEvent::New {
            community: REC.into(),
            channel: GROUP.into(),
            message_id: CALL.into(),
            sender_pseudonym: PK.into(),
            sequence: 1,
            timestamp: 7,
            body: Some("hello".into()),
            reply_to_message_id: None,
            automod_blurred: false,
        },
    ));
    assert_eq!(e.channel(), "community-event");
    assert!(e.audience().includes("community-browser"));
    let payload = e.payload().expect("serializes");
    let post = &payload["channelMessage"]["new"];
    assert_eq!(post["channel"], GROUP);
    assert_eq!(post["messageId"], CALL);
    assert_eq!(post["senderPseudonym"], PK);
    assert_eq!(post["body"], "hello");
    assert_eq!(post["automodBlurred"], false);
}
