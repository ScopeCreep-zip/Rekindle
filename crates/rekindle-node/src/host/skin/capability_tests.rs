//! Each capability from real state (SK2-G part (5) per feature): the
//! selection, the Host's downloads, and the world the worker reads.

use std::path::PathBuf;

use rekindle_types::dto::skin_action::Capability as C;
use rekindle_types::friend::{FriendState, FriendshipState, GameInfoState, UserStatus};

use rekindle_types::dto::transfer::{TransferRow, TransferState, TransferTarget};

use rekindle_ipc::protocol::IpcResponse;
use rekindle_types::dto::skin_action::SkinAction as A;
use rekindle_types::dto::skin_run::{CallOp, ShellOp, SkinRun};
use rekindle_types::subscription_events::{CallEvent, SubscriptionEvent};

use super::actions::{map, Mapped, Selected as Selection};
use super::capability::{allows, Group, LobbyCaps, Selected, SkinWorld};
use super::reads::Reads;
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
            lobby: None,
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
            lobby: None,
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
            lobby: None,
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
            lobby: None,
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
            lobby: None,
        },
    );
    assert!(!can(C::RenameRoom, &i, &w, g), "another member's room");
    w.groups.insert(
        "g1".into(),
        Group::Joined {
            others: 2,
            creator: true,
            lobby: None,
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

fn with_lobby(lobby: LobbyCaps) -> SkinWorld {
    let mut w = SkinWorld {
        own_game: Some(TF2),
        ..SkinWorld::default()
    };
    w.groups.insert(
        "g1".into(),
        Group::Joined {
            others: 2,
            creator: false,
            lobby: Some(lobby),
        },
    );
    w
}

/// SK2-G §5 (5): Join, Leave and Launch follow the Host's lobby verdicts,
/// and an open lobby stops a second Host.
#[test]
fn the_lobby_actions_follow_the_group_lobby() {
    let (i, g) = (Inputs::default(), Selected::Group { group_id: "g1" });
    let none = LobbyCaps {
        open: true,
        join: false,
        leave: false,
        launch: false,
    };
    let w = with_lobby(LobbyCaps { join: true, ..none });
    assert!(can(C::JoinGameLobby, &i, &w, g));
    assert!(!can(C::LeaveGameLobby, &i, &w, g) && !can(C::LaunchGameLobby, &i, &w, g));
    assert!(!can(C::HostGameLobby, &i, &w, g));
    let w = with_lobby(LobbyCaps {
        leave: true,
        ..none
    });
    assert!(can(C::LeaveGameLobby, &i, &w, g) && !can(C::JoinGameLobby, &i, &w, g));
    let w = with_lobby(LobbyCaps {
        launch: true,
        ..none
    });
    assert!(can(C::LaunchGameLobby, &i, &w, g));
    let w = with_lobby(LobbyCaps {
        open: false,
        ..none
    });
    assert!(can(C::HostGameLobby, &i, &w, g));
}

fn incoming(call_id: &str, from: &str) -> SubscriptionEvent {
    SubscriptionEvent::Call(CallEvent::Incoming {
        call_id: call_id.into(),
        from: from.into(),
        display_name: "K".into(),
        kind: "voice".into(),
        participants: Vec::new(),
        is_group: false,
        expires_at_ms: 0,
        group_id: None,
    })
}

fn outgoing(call_id: &str, to: &str) -> SubscriptionEvent {
    SubscriptionEvent::Call(CallEvent::Started {
        call_id: call_id.into(),
        kind: "voice".into(),
        peer_key: to.into(),
        peer_display_name: "K".into(),
        expires_at_ms: 0,
    })
}

/// Symbiosis pages its call tab by these (`ChatWindow.xml:280-284`):
/// `HostVoiceChat` is `ChatButtonCall` (`AppHostVoiceChat`), `VoiceChat`
/// is `ChatButtonAnswerCall` (`AppVoiceChat`, `Common.xml:450-456`), so
/// with no call the Call button shows, never Answer.
#[test]
fn host_voice_places_a_call_and_voice_answers_a_ringing_one() {
    let w = SkinWorld::default();
    let mut f = FriendState::new("k".into(), "K".into(), FriendshipState::Accepted);
    f.status = UserStatus::Online;
    let friend = Selected::Friend {
        key: "k",
        friend: Some(&f),
    };
    let mut i = Inputs::default();
    assert!(can(C::HostVoiceChat, &i, &w, friend));
    assert!(!can(C::VoiceChat, &i, &w, friend));
    assert!(i.ringing_from("k").is_none());

    i.fold(&incoming("in1", "k"));
    assert!(can(C::VoiceChat, &i, &w, friend));
    assert!(!can(C::HostVoiceChat, &i, &w, friend));
    assert!(!can(C::CancelVoiceChat, &i, &w, friend));
    assert_eq!(i.ringing_from("k").as_deref(), Some("in1"));
    assert!(i.ringing_from("other").is_none());

    let mut i = Inputs::default();
    i.fold(&outgoing("out1", "k"));
    assert!(can(C::CancelVoiceChat, &i, &w, friend));
    assert!(!can(C::VoiceChat, &i, &w, friend));
    assert!(!can(C::HostVoiceChat, &i, &w, friend));
    assert!(i.ringing_from("k").is_none());
}

/// `VoiceChat` answers the ringing call; it never places a new one.
#[test]
fn voice_chat_runs_answer_on_the_ringing_call() {
    let friend = || Selection {
        friend: Some("k"),
        ..Selection::default()
    };
    let reads = Reads {
        ringing_call: Some("in1".into()),
        ..Reads::default()
    };
    assert!(matches!(
        map(&A::VoiceChat, friend(), &reads),
        Ok(Mapped::Run(SkinRun::Shell(ShellOp::Call(CallOp::Answer { ref call_id })))) if call_id == "in1"
    ));
    assert!(matches!(
        map(&A::VoiceChat, friend(), &Reads::default()),
        Err(IpcResponse::Error { code: 409, .. })
    ));
    assert!(matches!(
        map(&A::HostVoiceChat, friend(), &Reads::default()),
        Ok(Mapped::Run(SkinRun::Shell(ShellOp::Call(CallOp::Voice { ref friend })))) if friend == "k"
    ));
}

fn group_incoming(
    call_id: &str,
    group_id: &str,
    from: &str,
    invited: &[&str],
) -> SubscriptionEvent {
    SubscriptionEvent::Call(CallEvent::Incoming {
        call_id: call_id.into(),
        from: from.into(),
        display_name: "A".into(),
        kind: "voice".into(),
        participants: invited.iter().map(|k| (*k).to_owned()).collect(),
        is_group: true,
        expires_at_ms: 0,
        group_id: Some(group_id.into()),
    })
}

fn joined_group(others: usize) -> Group {
    Group::Joined {
        others,
        creator: false,
        lobby: None,
    }
}

/// A group window pages its call tab by `HostVoiceChat`, `VoiceChat` and
/// `LeaveVoiceChat` (`TabbedGCCall`, Symbiosis
/// `Components/GroupChatWindow.xml:112-118`): Host starts the group's
/// call, Join joins its call in progress, Leave while we are in it.
#[test]
fn a_group_window_hosts_joins_and_leaves_its_group_call() {
    let mut w = SkinWorld::default();
    w.groups.insert("g1".into(), joined_group(2));
    let g = Selected::Group { group_id: "g1" };

    // No group call: Host can, Join and Leave cannot.
    let mut i = Inputs::default();
    assert!(can(C::HostVoiceChat, &i, &w, g));
    assert!(!can(C::VoiceChat, &i, &w, g));
    assert!(!can(C::LeaveVoiceChat, &i, &w, g));
    assert!(i.joinable_group_call("g1").is_none());

    // Another group's call leaves this group's tab on Host.
    i.fold(&group_incoming("other", "g9", "a", &["me", "c"]));
    assert!(can(C::HostVoiceChat, &i, &w, g));
    assert!(!can(C::VoiceChat, &i, &w, g));

    // The group's call in progress that we are not in: Join, not Host.
    i.fold(&group_incoming("gc1", "g1", "a", &["me", "b"]));
    assert!(can(C::VoiceChat, &i, &w, g));
    assert!(!can(C::HostVoiceChat, &i, &w, g));
    assert!(!can(C::LeaveVoiceChat, &i, &w, g));
    assert!(!can(C::CancelVoiceChat, &i, &w, g));
    assert_eq!(i.joinable_group_call("g1").as_deref(), Some("gc1"));

    // We are in it: Leave.
    i.fold(&SubscriptionEvent::Call(CallEvent::Connected {
        call_id: "gc1".into(),
        group_id: Some("g1".into()),
        direct: None,
    }));
    assert!(can(C::LeaveVoiceChat, &i, &w, g));
    assert!(!can(C::VoiceChat, &i, &w, g));
    assert!(!can(C::HostVoiceChat, &i, &w, g));
    assert!(i.joinable_group_call("g1").is_none());

    // A group we are only invited to has no call tab.
    w.groups.insert("g1".into(), Group::Pending);
    assert!(!can(C::LeaveVoiceChat, &i, &w, g));
    assert!(!can(C::HostVoiceChat, &i, &w, g));
}

/// Two group DMs of the same people each see only their own call: the
/// call is matched by the group id its signaling carries, never by its
/// members (plan E4.1).
#[test]
fn two_groups_with_the_same_members_each_match_only_their_own_call() {
    let mut w = SkinWorld::default();
    w.groups.insert("g1".into(), joined_group(2));
    w.groups.insert("g2".into(), joined_group(2));
    let mut i = Inputs::default();
    i.fold(&group_incoming("gc2", "g2", "a", &["me", "b"]));
    let (g1, g2) = (
        Selected::Group { group_id: "g1" },
        Selected::Group { group_id: "g2" },
    );
    assert_eq!(i.joinable_group_call("g2").as_deref(), Some("gc2"));
    assert!(i.joinable_group_call("g1").is_none());
    assert!(can(C::VoiceChat, &i, &w, g2));
    assert!(!can(C::VoiceChat, &i, &w, g1));
    assert!(can(C::HostVoiceChat, &i, &w, g1));
    // A buddy-list group call (no group id) belongs to neither.
    i.fold(&SubscriptionEvent::Call(CallEvent::Incoming {
        call_id: "adhoc".into(),
        from: "a".into(),
        display_name: "A".into(),
        kind: "voice".into(),
        participants: vec!["me".into(), "b".into()],
        is_group: true,
        expires_at_ms: 0,
        group_id: None,
    }));
    assert!(i.joinable_group_call("g1").is_none());
    assert!(can(C::HostVoiceChat, &i, &w, g1));
}

/// `GroupVoiceChat` (and a group window's `VoiceChat`) joins the group's
/// call in progress; a group window's `HostGroupVoiceChat` opens the
/// Start Group Call panel scoped to the group and prefilled with its
/// roster, the buddy list's opens the plain panel.
#[test]
fn group_voice_chat_joins_the_group_call_and_host_starts_one() {
    use super::reads::GroupCall;
    use rekindle_types::dto::skin_run::{GroupCallInvitee, ShellPanel};
    let roster = vec![
        GroupCallInvitee {
            pubkey: "b".into(),
            display_name: "Bea".into(),
        },
        GroupCallInvitee {
            pubkey: "c".into(),
            display_name: "Cy".into(),
        },
    ];
    let reads = |call_id: Option<&str>| Reads {
        group_call: Some(GroupCall {
            group_id: "g1".into(),
            group_name: "Raid".into(),
            members: roster.clone(),
            call_id: call_id.map(str::to_owned),
        }),
        ..Reads::default()
    };
    for action in [A::GroupVoiceChat, A::VoiceChat] {
        assert!(matches!(
            map(&action, Selection::default(), &reads(Some("gc1"))),
            Ok(Mapped::Run(SkinRun::Shell(ShellOp::Call(CallOp::JoinGroup { ref group_id, ref call_id }))))
                if group_id == "g1" && call_id == "gc1"
        ));
        assert!(matches!(
            map(&action, Selection::default(), &reads(None)),
            Err(IpcResponse::Error { code: 409, .. })
        ));
    }
    assert!(matches!(
        map(&A::GroupVoiceChat, Selection::default(), &Reads::default()),
        Err(IpcResponse::Error { code: 409, .. })
    ));
    // Symbiosis's group window draws `hostVoiceChat` on its Host button
    // (`ChatButtonCall`): it starts the group's call, never a 1:1 call.
    for action in [A::HostGroupVoiceChat, A::HostVoiceChat] {
        assert!(matches!(
            map(&action, Selection::default(), &reads(None)),
            Ok(Mapped::Run(SkinRun::Shell(ShellOp::StartGroupCall { ref group_id, ref group_name, ref members })))
                if group_id == "g1" && group_name == "Raid" && *members == roster
        ));
    }
    // A 1:1 window (no group read) keeps the 1:1 call.
    let bob = Selection {
        friend: Some("bob"),
        ..Selection::default()
    };
    assert!(matches!(
        map(&A::HostVoiceChat, bob, &Reads::default()),
        Ok(Mapped::Run(SkinRun::Shell(ShellOp::Call(CallOp::Voice { ref friend })))) if friend == "bob"
    ));
    assert!(matches!(
        map(
            &A::HostGroupVoiceChat,
            Selection::default(),
            &Reads::default()
        ),
        Ok(Mapped::Run(SkinRun::Shell(ShellOp::OpenPanel(
            ShellPanel::StartGroupCall
        ))))
    ));
}
