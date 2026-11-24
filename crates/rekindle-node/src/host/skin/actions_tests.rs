//! The skin action table (plan SK2-A4, design §2.2): each feature action
//! yields the request today's handler sends, a selection-dependent one the
//! Host's selection, and a feature gap `NotBuiltYet`.

use rekindle_ipc::protocol::{GroupDmRequest, IpcRequest, IpcResponse, ServersRequest};
use rekindle_types::dto::local_search::LocalSearchScope;
use rekindle_types::dto::skin_action::{
    EngineMenu, EngineView, PackUrl, SkinAction as A, SkinSettingsTab,
};
use rekindle_types::dto::skin_run::{CallOp, ShellOp, ShellPanel, SkinRun, SkinSelection};
use rekindle_types::presence::SessionStatus;

use super::actions::{addressed, map, resolve, Launchers, Mapped, Resolved, Selected, Views};
use super::reads::Reads;
use super::rooms::{Room, Rooms};
use super::SkinState;

const SELECTED_FRIEND_KEY: &str = "bobkey";

fn friend_selected() -> Selected<'static> {
    Selected {
        friend: Some(SELECTED_FRIEND_KEY),
        ..Selected::default()
    }
}

fn transfer_selected(t: &(String, rekindle_types::dto::transfer::TransferTarget)) -> Selected<'_> {
    Selected {
        transfer: Some(t),
        ..Selected::default()
    }
}

fn run(action: &A) -> SkinRun {
    match map(action, friend_selected(), &Reads::default()) {
        Ok(Mapped::Run(r)) => r,
        other => panic!("{action:?}: {other:?}"),
    }
}

fn request(action: &A) -> IpcRequest {
    match map(action, friend_selected(), &Reads::default()) {
        Ok(Mapped::Request(r)) => *r,
        other => panic!("{action:?}: {other:?}"),
    }
}

#[test]
fn feature_actions_yield_the_handlers_requests() {
    assert!(matches!(request(&A::LogOff), IpcRequest::Logout));
    assert!(matches!(
        request(&A::RefreshServer),
        IpcRequest::Servers(ServersRequest::Refresh)
    ));
    assert!(matches!(
        request(&A::SetStatus(SessionStatus::Away)),
        IpcRequest::PresenceSet {
            status: SessionStatus::Away
        }
    ));
    assert!(matches!(
        request(&A::LeaveVoiceChat),
        IpcRequest::VoiceLeave
    ));
    let url = A::LaunchUrl(PackUrl {
        url: "https://example.org/".into(),
    });
    assert!(matches!(request(&url), IpcRequest::LinkOpen { url } if url == "https://example.org/"));
}

#[test]
fn shell_actions_yield_their_operation_on_the_selected_friend() {
    let friend = SELECTED_FRIEND_KEY.to_owned();
    for (action, op) in [
        (
            A::ChatFriend,
            ShellOp::OpenChat {
                friend: friend.clone(),
            },
        ),
        (
            A::ProfileFriend,
            ShellOp::OpenProfile {
                friend: friend.clone(),
            },
        ),
        (
            A::HostVoiceChat,
            ShellOp::Call(CallOp::Voice {
                friend: friend.clone(),
            }),
        ),
        (A::CancelVoiceChat, ShellOp::Call(CallOp::End)),
        (A::EndVoiceChat, ShellOp::Call(CallOp::End)),
        (A::MuteSelf, ShellOp::Call(CallOp::Mute)),
        (A::UnmuteSelf, ShellOp::Call(CallOp::Unmute)),
        (A::AddFriend, ShellOp::OpenPanel(ShellPanel::AddFriend)),
        (A::NewChat, ShellOp::OpenPanel(ShellPanel::NewChat)),
        (A::NewGroup, ShellOp::OpenPanel(ShellPanel::NewGroup)),
        (
            A::CreateCommunity,
            ShellOp::OpenPanel(ShellPanel::CreateCommunity),
        ),
        (
            A::JoinCommunity,
            ShellOp::OpenPanel(ShellPanel::JoinCommunity),
        ),
        (
            A::ShowSettings(SkinSettingsTab::Privacy),
            ShellOp::ShowSettings(SkinSettingsTab::Privacy),
        ),
        (A::Quit, ShellOp::Quit),
        (
            A::Menu(EngineMenu::File),
            ShellOp::OpenMenu {
                menu: EngineMenu::File,
                context: false,
            },
        ),
    ] {
        assert_eq!(run(&action), SkinRun::Shell(op), "{action:?}");
    }
}

#[test]
fn a_selection_action_is_refused_while_nothing_is_selected() {
    let refused =
        map(&A::ChatFriend, Selected::default(), &Reads::default()).expect_err("no selection");
    assert!(matches!(refused, IpcResponse::Error { code: 409, .. }));
}

#[test]
fn unbuilt_actions_yield_not_built_yet() {
    for (action, message) in [
        (
            A::UploadSelected,
            "Uploading a picture or video isn't in Rekindle yet (plan E7.2 G14)",
        ),
        (
            A::DeleteSelected,
            "Deleting a picture or video isn't in Rekindle yet (plan E7.2 G14)",
        ),
    ] {
        assert_eq!(
            run(&action),
            SkinRun::NotBuiltYet {
                action: action.clone(),
                message: message.into(),
            }
        );
    }
    let menu = rekindle_types::dto::skin_action::MenuRegistry::FEATURE_GAPS[0];
    let gap_menu = A::Menu(menu);
    assert_eq!(
        run(&gap_menu),
        SkinRun::NotBuiltYet {
            action: gap_menu.clone(),
            message: format!("The {menu:?} menu isn't in Rekindle yet (plan E7.2 G9)"),
        }
    );
}

/// `ShowVoiceWindow` (plan E7.2 G-item A) shows the window the Host read
/// (the active call, else the voice session), and is refused with neither.
#[test]
fn show_voice_window_shows_the_call_or_voice_session() {
    use rekindle_types::dto::skin_run::CallTarget;
    for target in [
        CallTarget::Call {
            call_id: "c1".into(),
        },
        CallTarget::Voice {
            community: "VLD0:community".into(),
            channel: "ch".into(),
        },
    ] {
        let reads = Reads {
            show_call: Some(target.clone()),
            ..Reads::default()
        };
        assert!(matches!(
            map(&A::ShowVoiceWindow, Selected::default(), &reads),
            Ok(Mapped::Run(SkinRun::Shell(ShellOp::ShowCall { target: t }))) if t == target
        ));
    }
    assert!(matches!(
        map(&A::ShowVoiceWindow, Selected::default(), &Reads::default()),
        Err(IpcResponse::Error { code: 409, ref message, .. }) if message == "There's no call or voice session to show."
    ));
}

#[test]
fn resolve_takes_the_selection_from_the_host() {
    let skin = SkinState::default();
    let selections = &skin.selection;
    let chat = || IpcRequest::RunSkinAction {
        window: "buddy-list".into(),
        node: "ChatButton".into(),
        action: A::ChatFriend,
    };
    assert!(resolve(&skin, &Reads::default(), chat()).is_err());
    selections.set(
        "buddy-list",
        Some(SkinSelection::Friend {
            public_key: SELECTED_FRIEND_KEY.into(),
        }),
    );
    assert!(matches!(
        resolve(&skin, &Reads::default(), chat()),
        Ok(Resolved::Run {
            request: IpcRequest::RunSkinAction { .. },
            ..
        })
    ));
    let logoff = IpcRequest::RunSkinAction {
        window: "buddy-list".into(),
        node: "LogOff".into(),
        action: A::LogOff,
    };
    assert!(matches!(
        resolve(&skin, &Reads::default(), logoff),
        Ok(Resolved::Request(IpcRequest::Logout))
    ));
}

fn room(action: &A, friend: Option<&str>, rooms: &Rooms) -> Result<Mapped, IpcResponse> {
    let reads = Reads {
        rooms: rooms.clone(),
        ..Reads::default()
    };
    map(
        action,
        Selected {
            friend,
            ..Selected::default()
        },
        &reads,
    )
}

#[test]
fn room_actions_reach_their_group_dm_ops() {
    let g = "0f".repeat(16);
    let open = SkinRun::Shell(ShellOp::OpenGroupChat {
        group_id: g.clone(),
    });
    for action in [A::NewRoom, A::CreateRoom] {
        assert_eq!(
            run(&action),
            SkinRun::Shell(ShellOp::NewRoom {
                peer: Some(SELECTED_FRIEND_KEY.to_owned())
            })
        );
    }
    let joined = Rooms {
        selected: Some(Room::Joined(g.clone())),
        attached: None,
        lobby: None,
    };
    let pending = Rooms {
        selected: Some(Room::Pending(g.clone())),
        attached: None,
        lobby: None,
    };
    let attached = Rooms {
        selected: None,
        attached: Some(g.clone()),
        lobby: None,
    };
    for action in [A::EnterRoom, A::JoinRoom] {
        assert!(matches!(room(&action, None, &joined), Ok(Mapped::Run(r)) if r == open));
        assert!(matches!(
            room(&action, None, &pending),
            Ok(Mapped::Request(r)) if matches!(&*r, IpcRequest::GroupDm(GroupDmRequest::Accept { group_id }) if *group_id == g)
        ));
    }
    for action in [A::EnterAttachedRoom, A::EnterRoom] {
        assert!(
            matches!(room(&action, Some(SELECTED_FRIEND_KEY), &attached), Ok(Mapped::Run(r)) if r == open)
        );
    }
    for action in [A::EnterRoom, A::JoinRoom, A::EnterAttachedRoom] {
        assert!(matches!(
            room(&action, Some(SELECTED_FRIEND_KEY), &Rooms::default()),
            Err(IpcResponse::Error { code: 409, .. })
        ));
    }
    assert!(room(&A::EnterAttachedRoom, None, &pending).is_err());
}

/// `InviteUsersToRoom` (plan E7.2 G8) opens the selected joined room's
/// invite picker, and is refused on an invite or with no room.
#[test]
fn invite_users_to_room_opens_the_selected_rooms_picker() {
    let g = "0f".repeat(16);
    let joined = Rooms {
        selected: Some(Room::Joined(g.clone())),
        attached: None,
        lobby: None,
    };
    let picker = SkinRun::Shell(ShellOp::InviteToRoom {
        group_id: g.clone(),
    });
    assert!(
        matches!(room(&A::InviteUsersToRoom, None, &joined), Ok(Mapped::Run(r)) if r == picker)
    );
    let pending = Rooms {
        selected: Some(Room::Pending(g)),
        attached: None,
        lobby: None,
    };
    for rooms in [pending, Rooms::default()] {
        assert!(matches!(
            room(&A::InviteUsersToRoom, Some(SELECTED_FRIEND_KEY), &rooms),
            Err(IpcResponse::Error { code: 409, .. })
        ));
    }
}

/// The Downloads actions (plan E7.2 G12) reach the transfer requests on
/// the window's selected transfer, and are refused without one.
#[test]
fn each_download_action_reaches_its_transfer_request() {
    use rekindle_ipc::protocol::TransferRequest as T;
    let id = "0123456789abcdef0123456789abcdef";
    let selected = (
        id.to_owned(),
        rekindle_types::dto::transfer::TransferTarget::Channel {
            community: "VLD0:community".into(),
            channel_id: "ch".into(),
        },
    );
    let reached = |action: &A| match map(action, transfer_selected(&selected), &Reads::default()) {
        Ok(Mapped::Request(r)) => match *r {
            IpcRequest::Transfer(t) => t.name(),
            other => panic!("{action:?} reached {}", other.name()),
        },
        other => panic!("{action:?} mapped to {other:?}"),
    };
    assert_eq!(reached(&A::StartDownload), "TransferStart");
    assert_eq!(reached(&A::StopDownload), "TransferStop");
    assert_eq!(reached(&A::OpenDownload), "TransferOpen");
    assert_eq!(reached(&A::DownloadInfo), "TransferList");
    let Ok(Mapped::Request(start)) = map(
        &A::StartDownload,
        transfer_selected(&selected),
        &Reads::default(),
    ) else {
        unreachable!()
    };
    // The shell asks where to save, then sends the destination.
    assert!(matches!(
        *start,
        IpcRequest::Transfer(T::Start { ref attachment_id, ref target, dest: None })
            if attachment_id == id && *target == selected.1
    ));
    let Ok(Mapped::Request(stop)) = map(
        &A::StopDownload,
        transfer_selected(&selected),
        &Reads::default(),
    ) else {
        unreachable!()
    };
    assert!(matches!(
        *stop,
        IpcRequest::Transfer(T::Stop { ref attachment_id }) if attachment_id == id
    ));
    for action in [A::StartDownload, A::StopDownload, A::OpenDownload] {
        let refused = map(&action, friend_selected(), &Reads::default()).expect_err("no transfer");
        assert!(
            matches!(refused, IpcResponse::Error { code: 409, .. }),
            "{action:?}"
        );
    }
}

fn game(server: Option<&str>, joinable: bool) -> rekindle_types::friend::GameInfoState {
    rekindle_types::friend::GameInfoState {
        game_id: 4100,
        game_name: "Team Fortress 2".into(),
        server_info: None,
        elapsed_seconds: 0,
        server_address: server.map(str::to_owned),
        icon_key: None,
        joinable,
        joinable_reason: (!joinable).then_some(rekindle_types::friend::NotJoinableReason::NoServer),
    }
}

/// `JoinFriend` (lead decision 49) is the one launch path on the selected
/// friend's presence server, and is refused on the Host's `not_joinable`.
#[test]
fn join_friend_launches_the_selected_friends_presence_server() {
    use rekindle_types::game_server::JoinSource;
    let reads = Reads {
        friend_game: Some(game(Some("10.0.0.1:27015"), true)),
        ..Reads::default()
    };
    let Ok(Mapped::Request(r)) = map(&A::JoinFriend, friend_selected(), &reads) else {
        panic!("JoinFriend is a request")
    };
    assert!(matches!(
        *r,
        IpcRequest::GameLaunch { game_id: 4100, server: Some(ref s), source: JoinSource::FriendPresence }
            if s.as_str() == "10.0.0.1:27015"
    ));
    for refused in [
        Reads::default(),
        Reads {
            friend_game: Some(game(None, false)),
            ..Reads::default()
        },
        Reads {
            friend_game: Some(game(Some("not a server"), true)),
            ..Reads::default()
        },
    ] {
        assert!(matches!(
            map(&A::JoinFriend, friend_selected(), &refused),
            Err(IpcResponse::Error { code: 409, .. })
        ));
    }
    assert!(map(&A::JoinFriend, Selected::default(), &reads).is_err());
}

/// `AddServer` adds the selected server to our favourites; `DeleteSelected`
/// on a joined group leaves it.
#[test]
fn add_server_and_delete_room_reach_their_requests() {
    let server = (4100, "10.0.0.1:27015".to_owned());
    let on_server = Selected {
        server: Some(&server),
        ..Selected::default()
    };
    let Ok(Mapped::Request(r)) = map(&A::AddServer, on_server, &Reads::default()) else {
        panic!("AddServer is a request")
    };
    assert!(matches!(
        *r,
        IpcRequest::Servers(ServersRequest::FavouriteAdd { game_id: 4100, ref server, .. })
            if server.as_str() == "10.0.0.1:27015"
    ));
    assert!(matches!(
        map(&A::AddServer, friend_selected(), &Reads::default()),
        Err(IpcResponse::Error { code: 409, .. })
    ));
    let g = "0f".repeat(16);
    let joined = Reads {
        rooms: Rooms {
            selected: Some(Room::Joined(g.clone())),
            attached: None,
            lobby: None,
        },
        ..Reads::default()
    };
    assert!(matches!(
        map(&A::DeleteSelected, Selected::default(), &joined),
        Ok(Mapped::Request(r)) if matches!(&*r, IpcRequest::GroupDm(GroupDmRequest::Leave { group_id }) if *group_id == g)
    ));
}

/// A view switch pressed in a popup is its launcher's (plan SK2-N4): the
/// switch is addressed to the launcher and the popup runs nothing; a
/// window with no launcher, and a popup's other operations, keep theirs.
#[test]
fn a_popups_view_switch_is_addressed_to_its_launcher() {
    let launchers = Launchers::default();
    let popup = "popup-buddy-list-MainMenuPopup";
    let show = || SkinRun::Shell(ShellOp::ShowView(EngineView::Communities));
    let page = || SkinRun::Shell(ShellOp::GroupPage { group: 1, page: 3 });
    assert_eq!(addressed(&launchers, popup, show()), (show(), None));
    launchers.set(popup, Some("buddy-list"));
    for op in [show(), page()] {
        let SkinRun::Shell(sent) = op.clone() else {
            unreachable!()
        };
        assert_eq!(
            addressed(&launchers, popup, op),
            (
                SkinRun::Launcher {
                    window: "buddy-list".into()
                },
                Some(("buddy-list".to_owned(), sent))
            )
        );
    }
    assert_eq!(addressed(&launchers, "buddy-list", show()), (show(), None));
    let minimize = SkinRun::Shell(ShellOp::Minimize);
    assert_eq!(
        addressed(&launchers, popup, minimize.clone()),
        (minimize, None)
    );
    launchers.set(popup, None);
    assert_eq!(addressed(&launchers, popup, show()), (show(), None));
}

/// `SearchFor` pressed in `window` (SK2-G §8), as `resolve` maps it.
fn search_for(skin: &SkinState, window: &str) -> SkinRun {
    let pressed = IpcRequest::RunSkinAction {
        window: window.into(),
        node: "SearchButton".into(),
        action: A::SearchFor,
    };
    match resolve(skin, &Reads::default(), pressed) {
        Ok(Resolved::Run { run, .. }) => run,
        other => panic!("{other:?}"),
    }
}

fn open_search(scope: LocalSearchScope) -> SkinRun {
    SkinRun::Shell(ShellOp::OpenSearch { scope })
}

fn show(views: &Views, window: &str, view: EngineView) {
    views.record(window, &SkinRun::Shell(ShellOp::ShowView(view)), None);
}

#[test]
fn search_for_searches_the_windows_recorded_view() {
    let skin = SkinState::default();
    // The buddy list opens on its friends.
    assert_eq!(
        search_for(&skin, "buddy-list"),
        open_search(LocalSearchScope::Friends)
    );
    show(&skin.views, "buddy-list", EngineView::Servers);
    assert_eq!(
        search_for(&skin, "buddy-list"),
        open_search(LocalSearchScope::Servers)
    );
    show(&skin.views, "buddy-list", EngineView::Files);
    assert_eq!(
        search_for(&skin, "buddy-list"),
        open_search(LocalSearchScope::Files)
    );
    show(&skin.views, "buddy-list", EngineView::Friends);
    assert_eq!(
        search_for(&skin, "buddy-list"),
        open_search(LocalSearchScope::Friends)
    );
    // A view switch handed on from a popup is its launcher's view.
    skin.views.record(
        "popup-1",
        &SkinRun::Launcher {
            window: "buddy-list".into(),
        },
        Some(&("buddy-list".into(), ShellOp::ShowView(EngineView::Servers))),
    );
    skin.launchers.set("popup-1", Some("buddy-list"));
    assert_eq!(
        search_for(&skin, "buddy-list"),
        open_search(LocalSearchScope::Servers)
    );
    assert_eq!(
        search_for(&skin, "popup-1"),
        open_search(LocalSearchScope::Servers)
    );
    // The pictures and videos have nothing local to search yet (G14).
    show(&skin.views, "buddy-list", EngineView::Videos);
    assert_eq!(
        search_for(&skin, "buddy-list"),
        SkinRun::NotBuiltYet {
            action: A::SearchFor,
            message: "Searching pictures and videos isn't in Rekindle yet (plan E7.2 G14)".into(),
        }
    );
}

#[test]
fn search_for_in_a_chat_window_searches_its_conversation() {
    let skin = SkinState::default();
    skin.selection.set(
        "chat-0123456789abcdef",
        Some(SkinSelection::Friend {
            public_key: SELECTED_FRIEND_KEY.into(),
        }),
    );
    assert_eq!(
        search_for(&skin, "chat-0123456789abcdef"),
        open_search(LocalSearchScope::Conversation {
            conversation_id: SELECTED_FRIEND_KEY.into()
        })
    );
    let group = "0a".repeat(16);
    assert_eq!(
        search_for(&skin, &format!("chat-g{group}")),
        open_search(LocalSearchScope::Conversation {
            conversation_id: group
        })
    );
}
