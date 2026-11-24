//! Each capability from real state (SK2-G part (5) per feature): the
//! selection, the Host's downloads, and the world the worker reads.

use std::path::PathBuf;

use rekindle_types::dto::skin_action::Capability as C;
use rekindle_types::friend::{FriendState, FriendshipState, GameInfoState, UserStatus};

use rekindle_types::dto::transfer::{TransferRow, TransferState, TransferTarget};

use super::capability::{allows, Group, Selected, SkinWorld};
use super::signals::Inputs;

/// Team Fortress 2: a connect template.
const TF2: u32 = 4100;
/// Dota 2: no connect template; a Steam app id with Xfire's network
/// arguments (`+connect {addr}`), so `steam://run` puts it on a server.
const DOTA2: u32 = 4102;
/// No game in the database.
const UNKNOWN: u32 = 1;

fn can(c: C, inputs: &Inputs, world: &SkinWorld, selected: Selected<'_>) -> bool {
    allows(c, inputs, world, selected)
}

fn in_game(game_id: u32) -> FriendState {
    let mut f = FriendState::new("k".into(), "K".into(), FriendshipState::Accepted);
    f.status = UserStatus::Online;
    f.game_info = Some(GameInfoState {
        game_id,
        game_name: "G".into(),
        server_info: None,
        elapsed_seconds: 0,
        server_address: Some("10.0.0.1:27015".into()),
        icon_key: None,
        joinable: true,
        joinable_reason: None,
    });
    f
}

/// A file that exists for the length of the test.
fn temp_file(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("rekindle-cap-{}-{name}", std::process::id()));
    std::fs::write(&path, b"x").unwrap();
    path
}

#[test]
fn join_needs_a_game_we_can_launch() {
    let (i, w) = (Inputs::default(), SkinWorld::default());
    // Terraria (5002): a Steam app id but no way onto a server (plan
    // E7.2 G2), so it cannot be joined.
    for (game, launchable) in [(TF2, true), (DOTA2, true), (5002, false), (UNKNOWN, false)] {
        let f = in_game(game);
        let friend = Selected::Friend {
            key: "k",
            friend: Some(&f),
        };
        assert_eq!(can(C::Join, &i, &w, friend), launchable, "{game}");
        assert_eq!(can(C::JoinGameLobby, &i, &w, friend), launchable, "{game}");
        let server = Selected::Server {
            game_id: game,
            address: "10.0.0.1:27015",
        };
        assert_eq!(can(C::Join, &i, &w, server), launchable, "{game}");
    }
}

#[test]
fn add_server_unless_a_favourite_and_refresh_a_known_server() {
    let i = Inputs::default();
    let mut w = SkinWorld::default();
    let server = Selected::Server {
        game_id: TF2,
        address: "10.0.0.1:27015",
    };
    assert!(can(C::AddServer, &i, &w, server));
    assert!(!can(C::AddServer, &i, &w, Selected::Nothing));
    assert!(can(C::RefreshServer, &i, &w, server));
    assert!(!can(C::RefreshServer, &i, &w, Selected::Nothing));
    w.friend_servers = 1;
    assert!(can(C::RefreshServer, &i, &w, Selected::Nothing));
    w.friend_servers = 0;
    w.favourites.insert((TF2, "10.0.0.1:27015".into()));
    assert!(!can(C::AddServer, &i, &w, server));
    assert!(can(C::RefreshServer, &i, &w, Selected::Nothing));
}

fn transfer(state: TransferState, dest: &std::path::Path) -> TransferRow {
    TransferRow {
        attachment_id: "a1".into(),
        target: TransferTarget::Channel {
            community: "c".into(),
            channel_id: "ch".into(),
        },
        state,
        chunks_done: 0,
        chunk_count: 0,
        bytes_done: 0,
        total: 0,
        dest_name: "saved".into(),
        dest_path: dest.to_string_lossy().into_owned(),
        started_at: 0,
        finished_at: None,
    }
}

/// The capabilities follow the `transfers` row (plan E7.2 G12), not a
/// mark kept beside it.
#[test]
fn downloads_follow_the_selected_transfer_state() {
    let i = Inputs::default();
    let mut w = SkinWorld::default();
    let t = Selected::Transfer {
        attachment_id: "a1",
    };
    let state = |w: &SkinWorld| {
        [C::StartDownload, C::StopDownload, C::ExecuteDownload].map(|c| can(c, &i, w, t))
    };
    let saved = temp_file("saved");
    let set = |w: &mut SkinWorld, s| {
        w.transfers.insert("a1".into(), transfer(s, &saved));
    };
    assert_eq!(state(&w), [true, false, false], "offered, not fetched");
    set(&mut w, TransferState::Queued);
    assert_eq!(state(&w), [false, true, false], "queued");
    set(&mut w, TransferState::Active);
    assert_eq!(state(&w), [false, true, false], "active");
    set(&mut w, TransferState::Paused);
    assert_eq!(state(&w), [true, false, false], "paused");
    set(&mut w, TransferState::Failed);
    assert_eq!(state(&w), [true, false, false], "failed");
    set(&mut w, TransferState::Done);
    assert_eq!(state(&w), [false, false, true], "done");
    std::fs::remove_file(&saved).unwrap();
    assert_eq!(state(&w), [false, false, false], "done, file gone");
    assert!(!can(C::StartDownload, &i, &w, Selected::Nothing));
}

#[test]
fn a_lobby_is_hosted_in_a_group_with_a_member_while_in_a_launchable_game() {
    let i = Inputs::default();
    let mut w = SkinWorld::default();
    let g = Selected::Group { group_id: "g1" };
    w.groups.insert(
        "g1".into(),
        Group::Joined {
            others: 1,
            creator: false,
        },
    );
    w.own_game = Some(TF2);
    assert!(can(C::HostGameLobby, &i, &w, g));
    w.own_game = Some(UNKNOWN);
    assert!(!can(C::HostGameLobby, &i, &w, g));
    w.own_game = Some(TF2);
    w.groups.insert(
        "g1".into(),
        Group::Joined {
            others: 0,
            creator: false,
        },
    );
    assert!(!can(C::HostGameLobby, &i, &w, g));
    w.groups.insert("g1".into(), Group::Pending);
    assert!(!can(C::HostGameLobby, &i, &w, g));
    // We are in no lobby: nothing to join, leave or launch.
    w.groups.insert(
        "g1".into(),
        Group::Joined {
            others: 3,
            creator: false,
        },
    );
    for c in [C::JoinGameLobby, C::LeaveGameLobby, C::LaunchGameLobby] {
        assert!(!can(c, &i, &w, g), "{c:?}");
    }
}

#[test]
fn rooms_need_two_friends_to_create_and_a_known_group_to_enter() {
    let i = Inputs::default();
    let mut w = SkinWorld {
        accepted_friends: 1,
        ..SkinWorld::default()
    };
    assert!(!can(C::CreateRoom, &i, &w, Selected::Nothing));
    w.accepted_friends = 2;
    assert!(can(C::CreateRoom, &i, &w, Selected::Nothing));
    let g = Selected::Group { group_id: "g1" };
    assert!(!can(C::EnterRoom, &i, &w, g));
    w.groups.insert("g1".into(), Group::Pending);
    assert!(can(C::EnterRoom, &i, &w, g));
    assert!(!can(C::DeleteSelected, &i, &w, g), "an invite is not left");
    w.groups.insert(
        "g1".into(),
        Group::Joined {
            others: 2,
            creator: false,
        },
    );
    assert!(
        can(C::DeleteSelected, &i, &w, g),
        "a room is deleted by leaving"
    );
}

#[test]
fn only_the_creator_renames_a_room() {
    let i = Inputs::default();
    let mut w = SkinWorld::default();
    let g = Selected::Group { group_id: "g1" };
    assert!(!can(C::RenameRoom, &i, &w, g), "no such group");
    w.groups.insert("g1".into(), Group::Pending);
    assert!(!can(C::RenameRoom, &i, &w, g), "an invite");
    w.groups.insert(
        "g1".into(),
        Group::Joined {
            others: 2,
            creator: false,
        },
    );
    assert!(!can(C::RenameRoom, &i, &w, g), "another member's room");
    w.groups.insert(
        "g1".into(),
        Group::Joined {
            others: 2,
            creator: true,
        },
    );
    assert!(can(C::RenameRoom, &i, &w, g), "our room");
    assert!(!can(C::RenameRoom, &i, &w, Selected::Nothing));
}

#[test]
fn a_chat_enters_the_room_created_from_it() {
    let i = Inputs::default();
    let mut w = SkinWorld::default();
    let bob = Selected::Friend {
        key: "bob",
        friend: None,
    };
    assert!(!can(C::EnterRoom, &i, &w, bob));
    w.attached.insert("bob".into());
    assert!(can(C::EnterRoom, &i, &w, bob));
    let cat = Selected::Friend {
        key: "cat",
        friend: None,
    };
    assert!(!can(C::EnterRoom, &i, &w, cat));
}

#[test]
fn media_actions_need_the_file_and_a_share_target() {
    let i = Inputs::default();
    let mut w = SkinWorld::default();
    let file = temp_file("shot.png");
    let path = file.to_str().unwrap().to_owned();
    let m = Selected::Media { path: &path };
    assert!(can(C::OpenSelected, &i, &w, m));
    assert!(can(C::DeleteSelected, &i, &w, m));
    assert!(!can(C::UploadSelected, &i, &w, m), "nobody to share with");
    w.communities = 1;
    assert!(can(C::UploadSelected, &i, &w, m));
    std::fs::remove_file(&file).unwrap();
    for c in [C::OpenSelected, C::DeleteSelected, C::UploadSelected] {
        assert!(!can(c, &i, &w, m), "{c:?}");
    }
}
