//! The skin action table (plan SK2-A4, design §2.2): each feature action
//! yields the request today's handler sends, a selection-dependent one the
//! Host's selection, and a feature gap `NotBuiltYet`.

use rekindle_ipc::protocol::{IpcRequest, IpcResponse};
use rekindle_types::dto::skin_action::{EngineMenu, PackUrl, SkinAction as A, SkinSettingsTab};
use rekindle_types::dto::skin_run::{CallOp, ShellOp, ShellPanel, SkinRun, SkinSelection};
use rekindle_types::presence::SessionStatus;

use super::actions::{map, resolve, Mapped, Resolved, Selections};
use super::rooms::{Room, Rooms};

const SELECTED_FRIEND_KEY: &str = "bobkey";

fn run(action: &A) -> SkinRun {
    match map(action, Some(SELECTED_FRIEND_KEY), None, &Rooms::default()) {
        Ok(Mapped::Run(r)) => r,
        other => panic!("{action:?}: {other:?}"),
    }
}

fn request(action: &A) -> IpcRequest {
    match map(action, Some(SELECTED_FRIEND_KEY), None, &Rooms::default()) {
        Ok(Mapped::Request(r)) => *r,
        other => panic!("{action:?}: {other:?}"),
    }
}

#[test]
fn feature_actions_yield_the_handlers_requests() {
    assert!(matches!(request(&A::LogOff), IpcRequest::Logout));
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
            A::VoiceChat,
            ShellOp::Call(CallOp::Voice {
                friend: friend.clone(),
            }),
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
    let refused = map(&A::ChatFriend, None, None, &Rooms::default()).expect_err("no selection");
    assert!(matches!(refused, IpcResponse::Error { code: 409, .. }));
}

#[test]
fn gap_actions_yield_not_built_yet() {
    for action in [A::JoinFriend, A::AddServer, A::InviteUsersToRoom, A::SearchFor] {
        assert_eq!(
            run(&action),
            SkinRun::NotBuiltYet {
                action: action.clone()
            }
        );
    }
    let gap_menu = A::Menu(rekindle_types::dto::skin_action::MenuRegistry::FEATURE_GAPS[0]);
    assert_eq!(
        run(&gap_menu),
        SkinRun::NotBuiltYet {
            action: gap_menu.clone()
        }
    );
}

#[test]
fn resolve_takes_the_selection_from_the_host() {
    let selections = Selections::default();
    let chat = || IpcRequest::RunSkinAction {
        window: "buddy-list".into(),
        node: "ChatButton".into(),
        action: A::ChatFriend,
    };
    assert!(resolve(&selections, &Rooms::default(), chat()).is_err());
    selections.set(
        "buddy-list",
        Some(SkinSelection::Friend {
            public_key: SELECTED_FRIEND_KEY.into(),
        }),
    );
    assert!(matches!(
        resolve(&selections, &Rooms::default(), chat()),
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
        resolve(&selections, &Rooms::default(), logoff),
        Ok(Resolved::Request(IpcRequest::Logout))
    ));
}

fn room(action: &A, friend: Option<&str>, rooms: &Rooms) -> Result<Mapped, IpcResponse> {
    map(action, friend, None, rooms)
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
    };
    let pending = Rooms {
        selected: Some(Room::Pending(g.clone())),
        attached: None,
    };
    let attached = Rooms {
        selected: None,
        attached: Some(g.clone()),
    };
    for action in [A::EnterRoom, A::JoinRoom] {
        assert!(matches!(room(&action, None, &joined), Ok(Mapped::Run(r)) if r == open));
        assert!(matches!(
            room(&action, None, &pending),
            Ok(Mapped::Request(r)) if matches!(&*r, IpcRequest::GroupDmAccept { group_id } if *group_id == g)
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
    let reached = |action: &A| match map(action, None, Some(&selected), &Rooms::default()) {
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
    let Ok(Mapped::Request(start)) = map(&A::StartDownload, None, Some(&selected), &Rooms::default()) else {
        unreachable!()
    };
    // The shell asks where to save, then sends the destination.
    assert!(matches!(
        *start,
        IpcRequest::Transfer(T::TransferStart { ref attachment_id, ref target, dest: None })
            if attachment_id == id && *target == selected.1
    ));
    let Ok(Mapped::Request(stop)) = map(&A::StopDownload, None, Some(&selected), &Rooms::default()) else {
        unreachable!()
    };
    assert!(matches!(
        *stop,
        IpcRequest::Transfer(T::TransferStop { ref attachment_id }) if attachment_id == id
    ));
    for action in [A::StartDownload, A::StopDownload, A::OpenDownload] {
        let refused = map(&action, Some(SELECTED_FRIEND_KEY), None, &Rooms::default()).expect_err("no transfer");
        assert!(
            matches!(refused, IpcResponse::Error { code: 409, .. }),
            "{action:?}"
        );
    }
}
