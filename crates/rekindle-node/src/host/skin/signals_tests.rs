//! The skin signals' computation (plan SK2-A4 step 3).

use rekindle_types::dto::skin_action::Capability;
use rekindle_types::dto::skin_run::SkinSelection;
use rekindle_types::friend::{FriendState, FriendshipState, GameInfoState, UserStatus};
use rekindle_types::subscription_events::{CallEvent, SubscriptionEvent};

use super::actions::Selections;
use super::capability::{Selected, SkinWorld};
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
    let selected = Selected::Friend {
        key: SELECTED_KEY,
        friend: Some(f),
    };
    let set = compute(
        "buddy-list",
        &Inputs::default(),
        &SkinWorld::default(),
        selected,
    );
    assert_ne!(
        set.can.contains(&capability),
        set.cannot.contains(&capability)
    );
    set.can.contains(&capability)
}

#[test]
fn capabilities_follow_the_selected_friend_offline_online_in_game() {
    let offline = friend(UserStatus::Offline, None);
    assert!(!can(&offline, Capability::VoiceChat));
    assert!(!can(&offline, Capability::Join));
    assert!(can(&offline, Capability::Chat));
    let online = friend(UserStatus::Online, None);
    assert!(can(&online, Capability::VoiceChat));
    assert!(!can(&online, Capability::Join));
    let in_game = friend(UserStatus::Online, Some("10.0.0.1:27960"));
    assert!(can(&in_game, Capability::Join));
    assert!(!can(&in_game, Capability::AddFriend));
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
        Capability::AddFriend,
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
    assert!(sets["chat-online"].can.contains(&Capability::VoiceChat));
    assert!(sets["chat-offline"].cannot.contains(&Capability::VoiceChat));
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
