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
            retry: None,
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
    // Our own stored row (plan E6.11): its chat window and the buddy list.
    let own = sub(SubscriptionEvent::ChannelMessage(
        ChannelMessageEvent::DirectMessageSent {
            conversation_id: PK.into(),
            reply_to_id: None,
            message: rekindle_types::dto::dm::DmMessage {
                id: 1,
                message_id: Some("01900000-0000-7000-8000-000000000002".into()),
                sender_id: "me".into(),
                body: "hi".into(),
                timestamp: 1,
                is_own: true,
                send_state: rekindle_types::message::SendState::Queued,
                history_gap_through: None,
                expire_seconds: 0,
                expire_at: None,
                timer_notice_seconds: None,
            },
        },
    ));
    assert!(own.includes("buddy-list"));
    assert!(own.includes("chat-3f1a9c0b7e2d4f6a"));
    assert!(!own.includes("chat-0000000000000000"));
    assert!(!own.includes("community-browser"));
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
    // Our own stored group row (plan E6.11): its group window and the
    // buddy list, never another chat window.
    let own = sub(SubscriptionEvent::GroupDm(
        rekindle_types::subscription_events::GroupDmEvent::MessageSent {
            group_id: GROUP.into(),
            reply_to_id: None,
            message: rekindle_types::dto::dm::DmMessage {
                id: 1,
                message_id: Some("01900000-0000-7000-8000-000000000003".into()),
                sender_id: "me".into(),
                body: "hi".into(),
                timestamp: 1,
                is_own: true,
                send_state: rekindle_types::message::SendState::Queued,
                history_gap_through: None,
                expire_seconds: 0,
                expire_at: None,
                timer_notice_seconds: None,
            },
        },
    ));
    assert!(own.includes(&labels::conversation_label(GROUP).unwrap()));
    assert!(own.includes("buddy-list"));
    assert!(!own.includes("chat-3f1a9c0b7e2d4f6a"));
    assert!(!own.includes("community-browser"));
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

/// Lead decision (46), plan E3.2 S7s.4c: a transfer's link request asking
/// a co-creator reaches the buddy list, as the successor offer it answers
/// does (decision (33)), so the creator sees it without the community
/// window open; every community window gets both too.
#[test]
fn a_successor_link_request_reaches_the_buddy_list_as_the_offer_does() {
    use rekindle_types::subscription_events::GovernanceEvent;
    let offer = sub(SubscriptionEvent::Governance(
        GovernanceEvent::CommunitySuccessorOffered {
            community: REC.into(),
            new_anchor: REC.into(),
            signer: PK.into(),
            reason: rekindle_types::governance::SuccessorReason::Transfer,
            creators: vec![PK.into()],
        },
    ));
    let request = sub(SubscriptionEvent::Governance(
        GovernanceEvent::SuccessorLinkRequested {
            community: REC.into(),
            request: "8".repeat(64),
            recipient: PK.into(),
            new_anchor: REC.into(),
        },
    ));
    for a in [&offer, &request] {
        assert!(a.includes("buddy-list"));
        assert!(a.includes("community-browser"));
        assert!(!a.includes("settings"));
        assert!(!a.includes("chat-3f1a9c0b7e2d4f6a"));
    }
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

/// Lead decision 58: our own mute names no call, so it reaches the buddy
/// list and every call and chat window, and no community window.
#[test]
fn our_own_mute_reaches_the_buddy_list_call_and_chat_windows() {
    let a = sub(SubscriptionEvent::Call(CallEvent::SelfMuteChanged {
        muted: true,
        timestamp_ms: 1,
    }));
    assert!(a.includes("buddy-list"));
    assert!(a.includes("call-0123456789ab"));
    assert!(a.includes("chat-3f1a9c0b7e2d4f6a"));
    assert!(!a.includes("community-browser"));
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
            images: Vec::new(),
            palette: Vec::new(),
            shells: Vec::new(),
            title_logo: None,
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

/// Plan SK2-T1: a skin a window could not apply reaches that window, by
/// label, and Settings; no other window.
#[test]
fn a_skin_apply_failure_reaches_its_window_and_settings_only() {
    let e = WebviewEvent::Subscription(SubscriptionEvent::System(SystemEvent::SkinApplyFailed {
        window: "chat-0123456789ab".into(),
        skin: "builtin:xfire-classic".into(),
        cause: rekindle_types::dto::skin_library::SkinApplyFailure::SheetError {
            detail: "failed to load".into(),
        },
        reverted: false,
    }));
    assert_eq!(e.channel(), "skin-apply-event");
    let audience = e.audience();
    assert!(audience.includes("chat-0123456789ab"));
    assert!(audience.includes("settings"));
    for label in [
        "login",
        "buddy-list",
        "chat-ba9876543210",
        "community-x",
        "profile-x",
    ] {
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
            undecryptable: None,
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

#[test]
fn a_window_s_skin_signals_reach_that_window_only() {
    let e = WebviewEvent::Subscription(SubscriptionEvent::System(SystemEvent::SkinSignals {
        window: "chat-0123456789ab".into(),
        set: rekindle_types::dto::skin_signals::SkinSignalSet::default(),
    }));
    assert_eq!(e.channel(), "skin-signals-event");
    let audience = e.audience();
    assert!(audience.includes("chat-0123456789ab"));
    for label in ["chat-fedcba987654", "buddy-list", "settings", "login"] {
        assert!(!audience.includes(label), "{label}");
    }
}

/// A view switch pressed in a popup reaches its launcher alone (plan
/// SK2-N4), never the popup that was pressed.
#[test]
fn a_popup_s_view_switch_reaches_its_launcher_only() {
    let e = WebviewEvent::Subscription(SubscriptionEvent::System(SystemEvent::SkinShell {
        window: "buddy-list".into(),
        op: rekindle_types::dto::skin_run::ShellOp::ShowView(
            rekindle_types::dto::skin_action::EngineView::Communities,
        ),
    }));
    assert_eq!(e.channel(), "skin-shell-event");
    let audience = e.audience();
    assert!(audience.includes("buddy-list"));
    for label in [
        "popup-buddy-list-MainMenuPopup",
        "chat-fedcba987654",
        "settings",
    ] {
        assert!(!audience.includes(label), "{label}");
    }
}

/// The Downloads view is the buddy list's (plan E7.2 G12): a transfer's
/// progress reaches it, on its own channel, and no other window.
#[test]
fn transfers_reach_the_buddy_list_on_their_own_channel() {
    let e = WebviewEvent::Subscription(SubscriptionEvent::Transfer(
        rekindle_types::subscription_events::TransferEvent::Progress {
            attachment_id: "att".into(),
            chunks_done: 1,
            chunk_count: 4,
            bytes_done: 262_144,
            total: 1_000_000,
        },
    ));
    assert_eq!(e.channel(), "transfer-event");
    let audience = e.audience();
    assert!(audience.includes("buddy-list"));
    for label in ["chat-0123456789ab", "settings", "login"] {
        assert!(!audience.includes(label), "{label}");
    }
}

/// The Servers view (plan E7.2 G3) reaches the buddy list's Servers tab
/// and every chat window's skin tree, on its own channel, and no other
/// window.
#[test]
fn the_servers_view_reaches_the_buddy_list_and_chat_windows_only() {
    let e =
        WebviewEvent::Subscription(SubscriptionEvent::System(SystemEvent::ServersViewChanged {
            view: rekindle_types::dto::servers::ServersView {
                categories: vec![],
                refresh: rekindle_types::dto::servers::ServerVerdict::Allowed,
            },
        }));
    assert_eq!(e.channel(), "servers-event");
    let audience = e.audience();
    for label in ["buddy-list", "chat-0123456789ab", "chat-fedcba987654"] {
        assert!(audience.includes(label), "{label}");
    }
    for label in [
        "settings",
        "login",
        "community-0123456789ab",
        "call-0123456789ab",
    ] {
        assert!(!audience.includes(label), "{label}");
    }
}

/// Plan RV1: the route pause and the deaths warning are Device-scoped and
/// reach the buddy list (and every post-login window, never login) on
/// `network-status`, where the frontend's network store reads them.
#[test]
fn route_pause_resume_and_deaths_warning_reach_the_buddy_list_on_network_status() {
    use rekindle_types::subscription_events::{
        EventScope, NetworkEvent, OwnRouteClass, RoutePauseCause,
    };
    for e in [
        SubscriptionEvent::Network(NetworkEvent::RoutePaused {
            cause: RoutePauseCause::Unreadable,
        }),
        SubscriptionEvent::Network(NetworkEvent::RouteResumed {}),
        SubscriptionEvent::Network(NetworkEvent::RouteDeathsAboveBaseline {
            class: OwnRouteClass::General,
            deaths: 4,
            young: 1,
            window_secs: 3600,
            baseline: 3,
        }),
    ] {
        assert!(matches!(e.scope(), EventScope::Device));
        let a = sub(e.clone());
        assert!(a.includes("buddy-list"));
        assert!(a.includes("settings"));
        assert!(!a.includes("login"));
        assert_eq!(WebviewEvent::Subscription(e).channel(), "network-status");
    }
}
