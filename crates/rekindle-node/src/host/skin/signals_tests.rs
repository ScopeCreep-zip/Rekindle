//! The skin signals' computation (plan SK2-A4 step 3).

use rekindle_ipc::protocol::{IpcRequest, IpcResponse};
use rekindle_ipc::{Caller, HostApi};
use rekindle_types::dto::skin_action::{Capability, SkinAction};
use rekindle_types::dto::skin_run::{CallTarget, ShellOp, ShellPanel, SkinRun, SkinSelection};
use rekindle_types::friend::{FriendState, FriendshipState, GameInfoState, UserStatus};
use rekindle_types::subscription_events::{
    CallEvent, NetworkEvent, NotificationEvent, OwnRecordsPhase, StreamFrame, SubscriptionEvent,
    SystemEvent, VoiceEvent, VoiceScope,
};

use super::actions::{Launchers, Selections};
use rekindle_types::dto::skin_signals::{DisableCause, DisableSignal, SkinSignalSet};
use rekindle_types::subscription_events::RouteAvailability;

use super::capability::{refusal, Selected, SkinWorld};
use super::signals::{compute, window_sets, Inputs};

const SELECTED_KEY: &str = "selectedkey";
/// Team Fortress 2: launchable (a connect template).
const TF2: u32 = 4100;

fn friend(status: UserStatus, server: Option<&str>) -> FriendState {
    let mut f = FriendState::new(SELECTED_KEY.into(), "Sel".into(), FriendshipState::Accepted);
    f.status = status;
    f.game_info = server.map(|s| GameInfoState {
        game_id: TF2,
        game_name: "Quake".into(),
        server_info: None,
        elapsed_seconds: 0,
        server_address: Some(s.into()),
        icon_key: None,
        joinable: true,
        joinable_reason: None,
    });
    f
}

fn can(f: &FriendState, capability: Capability) -> bool {
    can_in(&SkinWorld::default(), f, capability)
}

fn can_in(world: &SkinWorld, f: &FriendState, capability: Capability) -> bool {
    let selected = Selected::Friend {
        key: SELECTED_KEY,
        friend: Some(f),
    };
    let set = compute("buddy-list", &Inputs::default(), world, selected);
    assert_ne!(
        set.can.contains(&capability),
        set.cannot.contains(&capability)
    );
    set.can.contains(&capability)
}

#[test]
fn capabilities_follow_the_selected_friend_offline_online_in_game() {
    let offline = friend(UserStatus::Offline, None);
    assert!(!can(&offline, Capability::HostVoiceChat));
    assert!(!can(&offline, Capability::Join));
    assert!(can(&offline, Capability::Chat));
    let online = friend(UserStatus::Online, None);
    assert!(can(&online, Capability::HostVoiceChat));
    assert!(!can(&online, Capability::VoiceChat));
    assert!(!can(&online, Capability::Join));
    let in_game = friend(UserStatus::Online, Some("10.0.0.1:27960"));
    assert!(can(&in_game, Capability::Join));
    // Plan SKC1: Add Friend holds with a friend selected, in game, while
    // our records are published (it was `cannot` for any friend before).
    let held = SkinWorld {
        records_held: true,
        ..SkinWorld::default()
    };
    assert!(can_in(&held, &in_game, Capability::AddFriend));
}

/// Plan SKC1: Add Friend follows the own-records stage alone, never the
/// selection: Xfire's button works at any time while signed in.
#[test]
fn add_friend_follows_the_published_records_not_the_selection() {
    let held = SkinWorld {
        records_held: true,
        ..SkinWorld::default()
    };
    let unheld = SkinWorld::default();
    let in_game = friend(UserStatus::Online, Some("10.0.0.1:27960"));
    let offline = friend(UserStatus::Offline, None);
    let selections = [
        Selected::Nothing,
        Selected::Friend {
            key: SELECTED_KEY,
            friend: Some(&in_game),
        },
        Selected::Friend {
            key: SELECTED_KEY,
            friend: Some(&offline),
        },
        Selected::Friend {
            key: SELECTED_KEY,
            friend: None,
        },
        Selected::Server {
            game_id: TF2,
            address: "10.0.0.1:27960",
        },
    ];
    for selected in selections {
        let inputs = Inputs::default();
        let set = compute("buddy-list", &inputs, &held, selected);
        assert!(set.can.contains(&Capability::AddFriend), "{selected:?}");
        let set = compute("buddy-list", &inputs, &unheld, selected);
        assert!(set.cannot.contains(&Capability::AddFriend), "{selected:?}");
    }
}

/// Plan SKC1: every own-records stage change on the stream recomputes
/// the signals, so the change of `records_held` is pushed.
#[test]
fn an_own_records_stage_event_recomputes_the_signals() {
    let mut inputs = Inputs::default();
    for phase in [
        OwnRecordsPhase::Pending,
        OwnRecordsPhase::Held,
        OwnRecordsPhase::Failed,
    ] {
        let event =
            SubscriptionEvent::Network(NetworkEvent::OwnRecordsChanged { phase, error: None });
        assert!(inputs.fold(&event), "{phase:?}");
    }
}

#[test]
fn no_selection_cannot_the_friend_capabilities_only() {
    let set = compute(
        "buddy-list",
        &Inputs::default(),
        &SkinWorld::default(),
        Selected::Nothing,
    );
    for c in [
        Capability::Chat,
        Capability::Profile,
        Capability::Join,
        Capability::VoiceChat,
    ] {
        assert!(set.cannot.contains(&c), "{c:?}");
    }
    // With nothing selected and nothing known, these have nothing to act on.
    for c in [
        Capability::AddServer,
        Capability::RefreshServer,
        Capability::HostGameLobby,
        Capability::CreateRoom,
    ] {
        assert!(set.cannot.contains(&c), "{c:?}");
    }
}

#[test]
fn self_mute_follows_the_call_state() {
    let mut inputs = Inputs::default();
    let set = compute(
        "buddy-list",
        &inputs,
        &SkinWorld::default(),
        Selected::Nothing,
    );
    assert!(set.cannot.contains(&Capability::MuteSelf));
    assert!(set.cannot.contains(&Capability::UnmuteSelf));
    inputs.fold(&SubscriptionEvent::Call(CallEvent::Connected {
        call_id: "c1".into(),
        group_id: None,
        direct: None,
    }));
    let set = compute(
        "buddy-list",
        &inputs,
        &SkinWorld::default(),
        Selected::Nothing,
    );
    assert!(set.can.contains(&Capability::MuteSelf));
    assert!(set.cannot.contains(&Capability::UnmuteSelf));
    assert!(inputs.fold(&self_mute(true)));
    let set = compute(
        "buddy-list",
        &inputs,
        &SkinWorld::default(),
        Selected::Nothing,
    );
    assert!(set.cannot.contains(&Capability::MuteSelf));
    assert!(set.can.contains(&Capability::UnmuteSelf));
}

fn self_mute(muted: bool) -> SubscriptionEvent {
    SubscriptionEvent::Call(CallEvent::SelfMuteChanged {
        muted,
        timestamp_ms: 1,
    })
}

fn mute_caps(inputs: &Inputs) -> (bool, bool) {
    let set = compute(
        "buddy-list",
        inputs,
        &SkinWorld::default(),
        Selected::Nothing,
    );
    (
        set.can.contains(&Capability::MuteSelf),
        set.can.contains(&Capability::UnmuteSelf),
    )
}

/// Lead decision 58: a `SelfMuteChanged` on the stream flips the mute
/// capabilities both ways; a repeat of the same state changes nothing.
#[test]
fn a_self_mute_event_flips_the_mute_capabilities() {
    let mut inputs = Inputs::default();
    inputs.fold(&SubscriptionEvent::Call(CallEvent::Connected {
        call_id: "c1".into(),
        group_id: None,
        direct: None,
    }));
    assert_eq!(mute_caps(&inputs), (true, false));
    assert!(inputs.fold(&self_mute(true)));
    assert_eq!(mute_caps(&inputs), (false, true));
    assert!(
        !inputs.fold(&self_mute(true)),
        "the same state is no change"
    );
    assert!(inputs.fold(&self_mute(false)));
    assert_eq!(mute_caps(&inputs), (true, false));
}

/// The last call's end clears our mute: the next call starts unmuted.
#[test]
fn call_end_resets_the_self_mute() {
    let mut inputs = Inputs::default();
    let connected = SubscriptionEvent::Call(CallEvent::Connected {
        call_id: "c1".into(),
        group_id: None,
        direct: None,
    });
    inputs.fold(&connected);
    inputs.fold(&self_mute(true));
    assert_eq!(mute_caps(&inputs), (false, true));
    inputs.fold(&SubscriptionEvent::Call(CallEvent::Ended {
        call_id: "c1".into(),
        reason: "hangup".into(),
    }));
    assert_eq!(
        mute_caps(&inputs),
        (false, false),
        "no call, no mute control"
    );
    inputs.fold(&connected);
    assert_eq!(mute_caps(&inputs), (true, false));
}

#[test]
fn two_chat_windows_with_different_peers_get_different_sets() {
    let selections = Selections::default();
    let pick = |window: &str, key: &str| {
        selections.set(
            window,
            Some(SkinSelection::Friend {
                public_key: key.into(),
            }),
        );
    };
    pick("chat-online", "onlinekey");
    pick("chat-offline", "offlinekey");
    let sets = window_sets(
        &Inputs::default(),
        &selections,
        &Launchers::default(),
        &SkinWorld::default(),
        |k| {
            let status = if k == "onlinekey" {
                UserStatus::Online
            } else {
                UserStatus::Offline
            };
            let mut f = friend(status, None);
            f.public_key = k.into();
            Some(f)
        },
    );
    assert!(sets["chat-online"].can.contains(&Capability::HostVoiceChat));
    assert!(sets["chat-offline"]
        .cannot
        .contains(&Capability::HostVoiceChat));
}

#[test]
fn focus_changes_the_set() {
    let mut inputs = Inputs::default();
    let before = compute("chat-a", &inputs, &SkinWorld::default(), Selected::Nothing);
    inputs.set_focus("chat-a", true);
    let after = compute("chat-a", &inputs, &SkinWorld::default(), Selected::Nothing);
    assert!(!before.focused);
    assert!(after.focused);
    assert!(
        !compute(
            "buddy-list",
            &inputs,
            &SkinWorld::default(),
            Selected::Nothing
        )
        .focused
    );
}

const BUDDY_LIST: &str = "buddy-list";

async fn buddy_signals_when(
    events: &mut rekindle_events::EventStream,
    ready: impl Fn(&SkinSignalSet) -> bool,
) -> SkinSignalSet {
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while let Some(frame) = events.next().await {
            if let StreamFrame::Event(event) = frame {
                if let SubscriptionEvent::System(SystemEvent::SkinSignals { window, set }) =
                    event.event
                {
                    if window == BUDDY_LIST && ready(&set) {
                        return set;
                    }
                }
            }
        }
        panic!("the Host signal stream ended")
    })
    .await
    .expect("the next buddy-list signal set")
}

#[tokio::test]
async fn an_alert_stays_unread_across_focus_until_the_notifications_panel_opens() {
    use crate::host::test_fixtures::{create_ann, offline_host};

    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    create_ann(&host).await;
    let mut events = host.ctx.journal.subscribe(None);
    host.skin_window_focused(BUDDY_LIST, true);
    let initial = buddy_signals_when(&mut events, |set| set.focused).await;
    let alert = || {
        SubscriptionEvent::Notification(NotificationEvent::SystemAlert {
            id: host.ctx.alert_id(),
            title: "Signing stopped".into(),
            body: "Review this device".into(),
            subject: None,
        })
    };
    let open = || IpcRequest::RunSkinAction {
        window: BUDDY_LIST.into(),
        node: "ButtonNotificationsTab".into(),
        action: SkinAction::OpenNotifications,
    };
    // Some platforms raise a standing memory notice on unlock. Clear it
    // via the same action before publishing this test's alert.
    if initial.new_notification {
        assert!(matches!(
            host.call(Caller::shell(), open()).await,
            IpcResponse::Ok { .. }
        ));
        buddy_signals_when(&mut events, |set| !set.new_notification).await;
    }

    host.publish_standing(alert());
    let unread = buddy_signals_when(&mut events, |set| set.new_notification).await;
    assert!(unread.focused);
    for focused in [false, true] {
        host.skin_window_focused(BUDDY_LIST, focused);
        let set = buddy_signals_when(&mut events, |set| set.focused == focused).await;
        assert!(set.new_notification, "focus must preserve unread alerts");
    }

    let response = host.call(Caller::shell(), open()).await;
    let IpcResponse::Ok { data, .. } = response else {
        panic!("opening notifications: {response:?}");
    };
    assert_eq!(
        serde_json::from_value::<SkinRun>(data).unwrap(),
        SkinRun::Shell(ShellOp::OpenPanel(ShellPanel::Notifications))
    );
    buddy_signals_when(&mut events, |set| !set.new_notification).await;

    host.ctx.journal.resync();
    assert!(
        !host.ctx.journal.has_unread_alert(),
        "resync retains read status"
    );
    host.publish(alert());
    buddy_signals_when(&mut events, |set| set.new_notification).await;
    host.shutdown().await.unwrap();
}

fn voice(event: VoiceEvent) -> SubscriptionEvent {
    SubscriptionEvent::Voice(event)
}

fn community_scope() -> VoiceScope {
    VoiceScope::Community {
        community: "VLD0:community".into(),
        channel: "ch".into(),
    }
}

/// `ShowVoiceWindow`'s window (plan E7.2 G-item A): the active call
/// first, else our community voice session; our own `Left` ends the
/// session, a peer's does not; a DM scope is no voice window.
#[test]
fn show_call_takes_the_call_then_the_voice_session() {
    let mut inputs = Inputs::default();
    inputs.set_own(Some("ownkey".into()));
    assert_eq!(inputs.show_call(), None);
    inputs.fold(&voice(VoiceEvent::LocalJoined {
        scope: community_scope(),
        join_handshake: rekindle_types::subscription_events::JoinHandshake::Announced,
        media_ready: rekindle_types::subscription_events::VoiceMediaReady {
            ready: false,
            reason: "handshake-announced".into(),
        },
    }));
    let session = Some(CallTarget::Voice {
        community: "VLD0:community".into(),
        channel: "ch".into(),
    });
    assert_eq!(inputs.show_call(), session);
    inputs.fold(&SubscriptionEvent::Call(CallEvent::Connected {
        call_id: "c1".into(),
        group_id: None,
        direct: None,
    }));
    assert_eq!(
        inputs.show_call(),
        Some(CallTarget::Call {
            call_id: "c1".into()
        })
    );
    inputs.fold(&SubscriptionEvent::Call(CallEvent::Ended {
        call_id: "c1".into(),
        reason: "hangup".into(),
    }));
    assert_eq!(inputs.show_call(), session);
    inputs.fold(&voice(VoiceEvent::Left {
        scope: community_scope(),
        pseudonym: "peerkey".into(),
    }));
    assert_eq!(inputs.show_call(), session, "a peer's leave is not ours");
    inputs.fold(&voice(VoiceEvent::Left {
        scope: community_scope(),
        pseudonym: "ownkey".into(),
    }));
    assert_eq!(inputs.show_call(), None);
    inputs.fold(&voice(VoiceEvent::LocalJoined {
        scope: VoiceScope::Dm {
            peer_key: "peerkey".into(),
        },
        join_handshake: rekindle_types::subscription_events::JoinHandshake::Announced,
        media_ready: rekindle_types::subscription_events::VoiceMediaReady {
            ready: false,
            reason: "handshake-announced".into(),
        },
    }));
    assert_eq!(inputs.show_call(), None);
}

/// `Join` follows the Host's launch check (`not_joinable`, lead decision
/// 49), the one `JoinFriend` refuses on: a launchable game whose shared
/// server does not parse is `cannot` (NoServer).
#[test]
fn join_follows_the_hosts_not_joinable_check() {
    let unparsed = friend(UserStatus::Online, Some("not a server"));
    assert!(!can(&unparsed, Capability::Join));
    assert!(!can(&unparsed, Capability::JoinGameLobby));
}

/// Plan E7.7 CR1: every capability in `cannot` carries the Host's reason,
/// the words of its refusal, and no capability in `can` carries one.
#[test]
fn every_cannot_has_its_refusal_as_its_reason() {
    let f = friend(UserStatus::Offline, None);
    let inputs = Inputs::default();
    let world = SkinWorld::default();
    for selected in [
        Selected::Nothing,
        Selected::Friend {
            key: SELECTED_KEY,
            friend: Some(&f),
        },
        Selected::Server {
            game_id: TF2,
            address: "192.0.2.1:27015",
        },
        Selected::Transfer { attachment_id: "a" },
    ] {
        let set = compute("buddy-list", &inputs, &world, selected);
        assert!(!set.cannot.is_empty(), "{selected:?}");
        for c in &set.cannot {
            let reason = set.disabled_reasons.get(&DisableCause::Capability(*c));
            assert!(reason.is_some_and(|r| !r.is_empty()), "{c:?} has no reason");
            assert_eq!(
                reason,
                refusal(*c, &inputs, &world, selected).as_ref(),
                "{c:?}"
            );
        }
        for c in &set.can {
            assert!(!set
                .disabled_reasons
                .contains_key(&DisableCause::Capability(*c)));
        }
    }
}

/// Plan E7.7 CR1: off the network, `disconnected` carries its reason;
/// attached, it carries none.
#[test]
fn disconnected_has_its_reason() {
    let mut inputs = Inputs::default();
    let event = SubscriptionEvent::Network(NetworkEvent::AttachmentChanged {
        attachment_state: "detached".into(),
        is_attached: false,
        public_internet_ready: false,
        general_route: RouteAvailability::Idle,
        media_route: RouteAvailability::Idle,
        can_write: false,
    });
    assert!(inputs.fold(&event));
    let world = SkinWorld::default();
    let set = compute("buddy-list", &inputs, &world, Selected::Nothing);
    assert!(set.disconnected);
    let cause = DisableCause::Signal(DisableSignal::Disconnected);
    assert_eq!(
        set.disabled_reasons.get(&cause).map(String::as_str),
        Some("You're offline. Rekindle reconnects on its own.")
    );
    let online = compute("buddy-list", &Inputs::default(), &world, Selected::Nothing);
    assert!(!online.disabled_reasons.contains_key(&cause));
}
