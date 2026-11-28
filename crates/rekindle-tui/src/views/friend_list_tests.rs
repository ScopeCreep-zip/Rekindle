//! The friend list's keys: the highlighted entry, the cursor over headers,
//! and Join on the Host's `joinable` (plan E2.1, E7.15 IV3).

use super::*;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use rekindle_types::friend::FriendshipState;
use rekindle_types::friend::UserStatus;

fn friend(key: &str, status: UserStatus) -> Friend {
    Friend {
        public_key: key.to_string(),
        display_name: key.to_string(),
        nickname: None,
        status,
        status_message: None,
        game_info: None,
        group_id: None,
        group: None,
        group_label: rekindle_types::dto::friends::DEFAULT_FRIEND_GROUP.to_string(),
        last_seen_at: None,
        friendship_state: FriendshipState::Accepted,
        handshake_error: None,
        handshake_retrying: false,
        identity_pin_mismatch: false,
        group_dm_block: None,
    }
}

fn loaded() -> FriendListView {
    let mut view = FriendListView::new(true);
    view.on_command_result(CommandResult::FriendListLoaded {
        friends: vec![
            friend("bob", UserStatus::Offline),
            friend("alice", UserStatus::Online),
            friend("carol", UserStatus::Online),
        ],
    })
    .unwrap();
    view.on_command_result(CommandResult::FriendExtrasLoaded {
        requests: vec![PendingRequest {
            public_key: "frank".into(),
            display_name: "frank".into(),
            message: String::new(),
            received_at: 0,
        }],
        blocked: vec![BlockedUser {
            public_key: "mallory".into(),
            display_name: "mallory".into(),
            blocked_at: 0,
        }],
        invites: vec![OutgoingInvite {
            invite_id: "inv1".into(),
            url: String::new(),
            created_at: 0,
            expires_at: 0,
            status: "pending".into(),
            accepted_by: None,
        }],
    })
    .unwrap();
    view
}

/// A key as the TUI delivers it: the global keymap's action into the
/// view's `update`, or, when the keymap leaves it, the view's own handler.
fn drive(view: &mut FriendListView, code: KeyCode) -> Option<Action> {
    let key = KeyEvent::new(code, KeyModifiers::NONE);
    let keymap = crate::keybinds::KeymapStore::load().unwrap();
    match keymap.classify(key, crate::keybinds::KeymapContext::Default) {
        Some(action) => view.update(action).unwrap(),
        None => view.handle_focused_key(key),
    }
}

#[test]
fn enter_opens_the_selected_friends_dm() {
    let mut view = loaded();
    assert!(matches!(
        drive(&mut view, KeyCode::Enter),
        Some(Action::ShowDmThread { peer_key }) if peer_key == "alice"
    ));
}

#[test]
fn x_rejects_the_selected_request_and_r_still_refreshes() {
    let mut view = loaded();
    // Online(2) alice carol, Offline(1) bob, then frank's request.
    for _ in 0..3 {
        drive(&mut view, KeyCode::Char('j'));
    }
    assert!(matches!(
        drive(&mut view, KeyCode::Char('x')),
        Some(Action::RejectFriendRequest(key)) if key == "frank"
    ));
    assert!(matches!(
        drive(&mut view, KeyCode::Char('r')),
        Some(Action::ShowFriendList)
    ));
}

fn press(view: &mut FriendListView, c: char) -> Option<Action> {
    view.handle_focused_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
}

#[test]
fn keys_act_on_the_highlighted_entry() {
    let mut view = loaded();
    // Online(2) alice carol, Offline(1) bob, three one-entry sections.
    assert_eq!(view.rows.len(), 11);
    assert_eq!(view.list_state.selected(), Some(1));
    assert!(
        matches!(press(&mut view, 'd'), Some(Action::RemoveFriend { peer_key }) if peer_key == "alice")
    );
    press(&mut view, 'j');
    press(&mut view, 'j');
    // The Offline header is skipped.
    assert_eq!(view.list_state.selected(), Some(4));
    assert!(
        matches!(press(&mut view, 'd'), Some(Action::RemoveFriend { peer_key }) if peer_key == "bob")
    );
    press(&mut view, 'j');
    assert!(matches!(
        press(&mut view, 'a'),
        Some(Action::AcceptFriendRequest { public_key, .. }) if public_key == "frank"
    ));
    press(&mut view, 'j');
    assert!(matches!(
        press(&mut view, 'u'),
        Some(Action::UnblockUser { public_key }) if public_key == "mallory"
    ));
    press(&mut view, 'j');
    assert!(matches!(
        press(&mut view, 'c'),
        Some(Action::CancelInvite { invite_id }) if invite_id == "inv1"
    ));
    // Past the last entry the cursor stays; up skips headers back.
    press(&mut view, 'j');
    assert_eq!(view.list_state.selected(), Some(10));
    press(&mut view, 'k');
    assert_eq!(view.list_state.selected(), Some(8));
    view.update(Action::ScrollToTop).unwrap();
    assert_eq!(view.list_state.selected(), Some(1));
    press(&mut view, 'k');
    assert_eq!(view.list_state.selected(), Some(1));
    view.update(Action::ScrollToBottom).unwrap();
    assert_eq!(view.list_state.selected(), Some(10));
}

fn playing(server: Option<&str>, joinable: bool, reason: Option<NotJoinableReason>) -> Friend {
    let mut f = friend("alice", UserStatus::Online);
    f.game_info = Some(rekindle_types::friend::GameInfoState {
        game_id: 4181,
        game_name: "Counter-Strike 2".into(),
        server_info: None,
        elapsed_seconds: 0,
        server_address: server.map(str::to_owned),
        icon_key: None,
        joinable,
        joinable_reason: reason,
    });
    f
}

fn with(friend: Friend) -> FriendListView {
    let mut view = FriendListView::new(true);
    view.on_command_result(CommandResult::FriendListLoaded {
        friends: vec![friend],
    })
    .unwrap();
    view
}

#[test]
fn join_while_not_joinable_shows_the_hosts_reason_and_sends_nothing() {
    let cases = [
        (friend("alice", UserStatus::Online), "not in a game"),
        (
            playing(None, false, Some(NotJoinableReason::NoServer)),
            "not on a server",
        ),
        (
            playing(
                Some("10.0.0.1:27015"),
                false,
                Some(NotJoinableReason::CannotLaunch),
            ),
            "can't be launched here",
        ),
    ];
    for (f, reason) in cases {
        let mut view = with(f);
        let Some(Action::ShowToast { message, .. }) = press(&mut view, 'J') else {
            panic!("a disabled Join shows the reason and launches nothing");
        };
        assert_eq!(message, format!("Can't join alice: {reason}"));
    }
}

#[test]
fn join_while_joinable_launches_to_the_friends_server() {
    let mut view = with(playing(Some("10.0.0.1:27015"), true, None));
    let Some(Action::LaunchGame {
        game_id,
        server,
        source,
    }) = press(&mut view, 'J')
    else {
        panic!("Join launches the friend's game");
    };
    assert_eq!(
        (game_id, server.as_ref().map(ServerAddress::as_str), source),
        (4181, Some("10.0.0.1:27015"), JoinSource::FriendPresence)
    );
}

#[test]
fn a_reload_keeps_the_cursor_off_headers() {
    let mut view = loaded();
    view.update(Action::ScrollToBottom).unwrap();
    view.on_command_result(CommandResult::FriendExtrasLoaded {
        requests: Vec::new(),
        blocked: Vec::new(),
        invites: Vec::new(),
    })
    .unwrap();
    assert_eq!(view.rows.len(), 5);
    assert_eq!(view.list_state.selected(), Some(4));
    assert!(view.selected().is_some());
}
