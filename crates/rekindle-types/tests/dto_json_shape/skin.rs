//! `SkinList`'s entries in the golden DTO file.

use rekindle_types::dto::skin::{Insets, OriginalLicence, SkinCredits, SkinWindow};
use rekindle_types::dto::skin_action::{
    AnimateSpec, AppSignal, Capability, EngineMenu, EngineView, MenuRegistry, NodeState,
    NodeTrigger, OverlayPart, PackUrl, PopupRef, SkinAction, SkinEvent, SkinSettingsTab, SoundRef,
    TileRef,
};
use rekindle_types::dto::skin_library::{SkinApplyFailure, SkinEntry};
use rekindle_types::dto::skin_run::SkinSelection;
use rekindle_types::dto::skin_tree::{NodeRole, NodeSpan, SkinNode, SkinTree};
use rekindle_types::dto::skin_tree_role::Edge;
use rekindle_types::presence::SessionStatus;
use serde_json::Value;

use super::{pk, to};

/// `SkinList`'s entries (plan E7.7.3d): a built-in with every credit, a
/// library skin stating no licence and naming no one.
pub(super) fn samples(add: &mut impl FnMut(&str, Value)) {
    selection_samples(add);
    infoview_subject_samples(add);
    call_target_samples(add);
    // `SkinApplyFailed`'s cause, each kind (the image one is also in the
    // event golden).
    add(
        "skin.applyFailure.sheetError",
        to(SkinApplyFailure::SheetError {
            detail: "the stylesheet failed to load".into(),
        }),
    );
    add(
        "skin.entry.builtin",
        to(SkinEntry {
            id: "builtin:xfire-classic".into(),
            name: "Xfire".into(),
            imported_at: None,
            credits: Some(SkinCredits {
                authors: vec!["Xfire, Inc.".into()],
                contributors: vec!["Tim Brengle".into()],
                url: Some("http://www.xfire.com/".into()),
                licence: OriginalLicence::NoneStated,
            }),
            short_targets: 4,
            conforming: false,
        }),
    );
    add(
        "skin.entry.library",
        to(SkinEntry {
            id: format!("pkg:{}", pk('d')),
            name: "Night".into(),
            imported_at: Some(1_760_000_000_000),
            credits: Some(SkinCredits {
                authors: Vec::new(),
                contributors: Vec::new(),
                url: None,
                licence: OriginalLicence::Text("Free to share.".into()),
            }),
            short_targets: 0,
            conforming: true,
        }),
    );
    add(
        "skin.entry.uncredited",
        to(SkinEntry {
            id: format!("pkg:{}", pk('e')),
            name: "Plain".into(),
            imported_at: Some(1_760_000_000_001),
            credits: None,
            short_targets: 2,
            conforming: false,
        }),
    );
    actions(add);
}

/// Every skin action, event and signal shape (plan SK2-A1, design
/// `sk2a/design.md` §2.2), a node's trigger, and the menu registry.
fn actions(add: &mut impl FnMut(&str, Value)) {
    use SkinAction as A;
    let spec = AnimateSpec {
        delay: 4,
        repeat: None,
        action: Some(1),
    };
    let all = vec![
        A::DragWindow,
        A::Minimize,
        A::Maximize,
        A::Close,
        A::Resize(Edge::BottomRight),
        A::Menu(EngineMenu::File),
        A::ContextMenu(EngineMenu::Sys),
        A::ShowView(EngineView::Friends),
        A::GroupPage { group: 4, page: 1 },
        A::ShowTile(TileRef::This),
        A::HideTile(TileRef::Named("Lamp".into())),
        A::LaunchPopup(PopupRef {
            popup: "MainInfoViewPopup".into(),
        }),
        A::Animate(spec),
        A::AnimateStop,
        A::AnimationStart { group: 100, spec },
        A::AnimationStop { group: 100 },
        A::SetState(NodeState::Disabled),
        A::AddFriend,
        A::ChatFriend,
        A::ProfileFriend,
        A::ViewProfile,
        A::JoinFriend,
        A::SendMessage,
        A::VoiceChat,
        A::HostVoiceChat,
        A::LeaveVoiceChat,
        A::GroupVoiceChat,
        A::HostGroupVoiceChat,
        A::LeaveGroupVoiceChat,
        A::CancelVoiceChat,
        A::EndVoiceChat,
        A::MuteSelf,
        A::UnmuteSelf,
        A::ShowVoiceWindow,
        A::HostGameLobby,
        A::JoinGameLobby,
        A::LeaveGameLobby,
        A::LaunchGameLobby,
        A::AddServer,
        A::RefreshServer,
        A::StartDownload,
        A::StopDownload,
        A::OpenDownload,
        A::DownloadInfo,
        A::NewRoom,
        A::CreateRoom,
        A::EnterRoom,
        A::EnterAttachedRoom,
        A::JoinRoom,
        A::InviteUsersToRoom,
        A::DeleteSelected,
        A::OpenSelected,
        A::UploadSelected,
        A::SearchFor,
        A::LaunchUrl(PackUrl {
            url: "http://www.xfire.com/".into(),
        }),
        A::PlaySound(SoundRef {
            sound: "OfflineSound".into(),
        }),
        A::ShowWindow,
        A::HideWindow,
        A::DialogOk,
        A::DialogCancel,
        A::EnableOverlay(OverlayPart::Fps),
        A::DisableOverlay(OverlayPart::WebCam),
        A::UpdateClock,
        A::TackWindow,
        A::UntackWindow,
        A::OpenOverlayOptions,
        A::AttachedClose,
        A::BrowseBack,
        A::BrowseForward,
        A::NewChat,
        A::NewGroup,
        A::CreateCommunity,
        A::JoinCommunity,
        A::ShowSettings(SkinSettingsTab::Appearance),
        A::ShowShortcuts,
        A::ReportBug,
        A::SetStatus(SessionStatus::Invisible),
        A::LogOff,
        A::Quit,
        A::CopyKey,
        A::EditName,
        A::ShowSafetyNumber,
        A::ShowGameInvite,
        A::VideoChat,
        A::OpenNotifications,
    ];
    add("skin.action.all", super::to(all));
    let events = vec![
        SkinEvent::Click,
        SkinEvent::LeftDown,
        SkinEvent::Show(EngineView::Servers),
        SkinEvent::App(AppSignal::Can(Capability::Join)),
        SkinEvent::App(AppSignal::Cannot(Capability::VoiceChat)),
        SkinEvent::App(AppSignal::PrefEnabled(OverlayPart::Clock)),
        SkinEvent::App(AppSignal::Disconnected),
    ];
    add("skin.event.samples", super::to(events));
    add(
        "skin.trigger",
        super::to(NodeTrigger {
            on: SkinEvent::App(AppSignal::Cannot(Capability::Chat)),
            run: SkinAction::SetState(NodeState::Disabled),
        }),
    );
    add("skin.menuRegistry", super::to(MenuRegistry::rekindle()));
    shell(add);
}

/// A served shell (design §2.6): a tab control's node with its
/// `controlGroup`, a node without one, and Rekindle's `menus`.
fn shell(add: &mut impl FnMut(&str, Value)) {
    let fill = NodeSpan::Stretch { start: 0, end: 0 };
    let node = |id: &str, role, control_group, children| SkinNode {
        id: id.to_owned(),
        role,
        x: fill,
        y: fill,
        z: 0,
        art: None,
        text: None,
        label: None,
        inset: None,
        target: None,
        shows_with: None,
        control_group,
        triggers: Vec::new(),
        children,
    };
    let tree = SkinTree {
        outsets: Insets {
            top: 0,
            right: 0,
            bottom: 0,
            left: 0,
        },
        root: vec![node(
            "ButtonArea",
            NodeRole::ButtonBar,
            Some(1),
            vec![node("Plain", NodeRole::Group, None, Vec::new())],
        )],
    };
    add(
        "skin.shell",
        super::to(tree.shell(
            SkinWindow::Main,
            (368, 400),
            vec!["skin://app/0123/abcd.png".to_owned()],
        )),
    );
}

/// Every `SkinSelection` variant (design §2.2): its fields are camelCase
/// like every IPC struct, as the TS mirror (`dto_skin_run.ts`) sends them.
fn selection_samples(add: &mut impl FnMut(&str, Value)) {
    add(
        "skin.selection.friend",
        to(SkinSelection::Friend {
            public_key: pk('a'),
        }),
    );
    add(
        "skin.selection.server",
        to(SkinSelection::Server {
            game_id: 4100,
            address: "203.0.113.7:27015".into(),
        }),
    );
    add(
        "skin.selection.transfer",
        to(SkinSelection::Transfer {
            attachment_id: "att-1".into(),
            target: rekindle_types::dto::transfer::TransferTarget::Channel {
                community: "VLD0:community".into(),
                channel_id: "ch".into(),
            },
        }),
    );
    add(
        "skin.selection.media",
        to(SkinSelection::Media {
            path: "shot.png".into(),
        }),
    );
    add(
        "skin.selection.group",
        to(SkinSelection::Group {
            group_id: "g1".into(),
        }),
    );
    add(
        "skin.selection.friendGroup",
        to(SkinSelection::FriendGroup { group_id: Some(3) }),
    );
}

/// `ShowVoiceWindow`'s `ShellOp::ShowCall`, each `CallTarget` (plan E7.2
/// G-item A): camelCase fields, as the TS mirror reads them.
fn call_target_samples(add: &mut impl FnMut(&str, Value)) {
    use rekindle_types::dto::skin_run::{CallTarget, ShellOp};
    add(
        "skin.shellOp.showCall.call",
        to(ShellOp::ShowCall {
            target: CallTarget::Call {
                call_id: "c1".into(),
            },
        }),
    );
    add(
        "skin.shellOp.showCall.voice",
        to(ShellOp::ShowCall {
            target: CallTarget::Voice {
                community: "VLD0:community".into(),
                channel: "ch".into(),
            },
        }),
    );
    shell_op_samples(add);
}

/// The `ShellOp`s with multi-word fields and every `SkinRun` (plan E7.2):
/// fields camelCase like the variants, as `dto_skin_run.ts` reads them, so
/// a casing drift fails here.
fn shell_op_samples(add: &mut impl FnMut(&str, Value)) {
    use rekindle_types::dto::skin_run::{CallOp, ShellOp, SkinRun};
    add(
        "skin.shellOp.openGroupChat",
        to(ShellOp::OpenGroupChat {
            group_id: "g1".into(),
        }),
    );
    add(
        "skin.shellOp.inviteToRoom",
        to(ShellOp::InviteToRoom {
            group_id: "g1".into(),
        }),
    );
    add(
        "skin.shellOp.chooseDownloadDest",
        to(ShellOp::ChooseDownloadDest {
            attachment_id: "att-1".into(),
            target: rekindle_types::dto::transfer::TransferTarget::Channel {
                community: "VLD0:community".into(),
                channel_id: "ch".into(),
            },
            suggested_name: "notes.txt".into(),
        }),
    );
    add(
        "skin.shellOp.newRoom",
        to(ShellOp::NewRoom {
            peer: Some(pk('a')),
        }),
    );
    add(
        "skin.run.shell",
        to(SkinRun::Shell(ShellOp::Call(CallOp::Voice {
            friend: pk('a'),
        }))),
    );
    add(
        "skin.run.notBuiltYet",
        to(SkinRun::NotBuiltYet {
            action: SkinAction::SearchFor,
            message: "Search isn't in Rekindle yet (plan E7.2 G11)".into(),
        }),
    );
    add(
        "skin.run.launcher",
        to(SkinRun::Launcher {
            window: "buddy-list".into(),
        }),
    );
}

/// Every `InfoviewSubject` a window asks (plan E7.15 IV5): an Info popup
/// asks `selection`, a unit variant tagged like the rest.
fn infoview_subject_samples(add: &mut impl FnMut(&str, Value)) {
    use rekindle_types::dto::infoview::InfoviewSubject as S;
    add("infoview.subject.own", to(S::Own));
    add(
        "infoview.subject.friend",
        to(S::Friend {
            public_key: pk('a'),
        }),
    );
    add(
        "infoview.subject.friendGroup",
        to(S::FriendGroup { group_id: None }),
    );
    add(
        "infoview.subject.community",
        to(S::Community {
            community: "VLD0:community".into(),
        }),
    );
    add("infoview.subject.selection", to(S::Selection));
}
