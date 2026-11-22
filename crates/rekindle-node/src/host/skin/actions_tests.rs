//! The skin action table (plan SK2-A4, design §2.2): each feature action
//! yields the request today's handler sends, a selection-dependent one the
//! Host's selection, and a feature gap `NotBuiltYet`.

use rekindle_ipc::protocol::{IpcRequest, IpcResponse};
use rekindle_types::dto::skin_action::{EngineMenu, PackUrl, SkinAction as A, SkinSettingsTab};
use rekindle_types::dto::skin_layout::SkinWindow;
use rekindle_types::dto::skin_run::{CallOp, ShellOp, ShellPanel, SkinRun, SkinSelection};
use rekindle_types::presence::SessionStatus;

use super::actions::{map, resolve, Mapped, Selections};

const SELECTED_FRIEND_KEY: &str = "bobkey";

fn run(action: &A) -> SkinRun {
    match map(action, Some(SELECTED_FRIEND_KEY)) {
        Ok(Mapped::Run(r)) => r,
        other => panic!("{action:?}: {other:?}"),
    }
}

fn request(action: &A) -> IpcRequest {
    match map(action, Some(SELECTED_FRIEND_KEY)) {
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
    let refused = map(&A::ChatFriend, None).expect_err("no selection");
    assert!(matches!(refused, IpcResponse::Error { code: 409, .. }));
}

#[test]
fn gap_actions_yield_not_built_yet() {
    for action in [
        A::JoinFriend,
        A::AddServer,
        A::StartDownload,
        A::CreateRoom,
        A::SearchFor,
    ] {
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
        window: SkinWindow::Main,
        node: "ChatButton".into(),
        action: A::ChatFriend,
    };
    assert!(resolve(&selections, chat()).is_err());
    selections.set(
        SkinWindow::Main,
        Some(SkinSelection::Friend {
            public_key: SELECTED_FRIEND_KEY.into(),
        }),
    );
    assert!(matches!(
        resolve(&selections, chat()),
        Ok(IpcRequest::RunSkinAction { .. })
    ));
    let logoff = IpcRequest::RunSkinAction {
        window: SkinWindow::Main,
        node: "LogOff".into(),
        action: A::LogOff,
    };
    assert!(matches!(
        resolve(&selections, logoff),
        Ok(IpcRequest::Logout)
    ));
}
