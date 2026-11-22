//! The skin signals' computation (plan SK2-A4 step 3).

use rekindle_types::dto::skin_action::Capability;
use rekindle_types::dto::skin_layout::SkinWindow;
use rekindle_types::friend::{FriendState, FriendshipState, GameInfoState, UserStatus};

use super::signals::{compute, Inputs};

const SELECTED_KEY: &str = "selectedkey";

fn friend(status: UserStatus, server: Option<&str>) -> FriendState {
    let mut f = FriendState::new(SELECTED_KEY.into(), "Sel".into(), FriendshipState::Accepted);
    f.status = status;
    f.game_info = server.map(|s| GameInfoState {
        game_id: 1,
        game_name: "Quake".into(),
        server_info: None,
        elapsed_seconds: 0,
        server_address: Some(s.into()),
        icon_key: None,
    });
    f
}

fn can(f: &FriendState, capability: Capability) -> bool {
    let set = compute(
        SkinWindow::Main,
        &Inputs::default(),
        Some((SELECTED_KEY, Some(f))),
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
fn no_selection_cannot_anything() {
    let set = compute(SkinWindow::Main, &Inputs::default(), None);
    assert!(set.can.is_empty());
    assert_eq!(set.cannot.len(), 25);
}

#[test]
fn focus_changes_the_set() {
    let mut inputs = Inputs::default();
    let before = compute(SkinWindow::Chat, &inputs, None);
    inputs.set_focus(SkinWindow::Chat, true);
    let after = compute(SkinWindow::Chat, &inputs, None);
    assert!(!before.focused);
    assert!(after.focused);
    assert!(!compute(SkinWindow::Main, &inputs, None).focused);
}
