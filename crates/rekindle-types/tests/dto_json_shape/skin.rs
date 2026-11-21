//! `SkinList`'s entries in the golden DTO file.

use rekindle_types::dto::skin::{OriginalLicence, SkinCredits};
use rekindle_types::dto::skin_action::{
    AnimateSpec, AppSignal, Capability, EngineMenu, EngineView, MenuRegistry, NodeState,
    NodeTrigger, OverlayPart, PackUrl, PopupRef, SkinAction, SkinEvent, SkinSettingsTab, SoundRef,
    TileRef,
};
use rekindle_types::dto::skin_library::SkinEntry;
use rekindle_types::dto::skin_tree_role::Edge;
use rekindle_types::presence::SessionStatus;
use serde_json::Value;

use super::{pk, to};

/// `SkinList`'s entries (plan E7.7.3d): a built-in with every credit, a
/// library skin stating no licence and naming no one.
pub(super) fn samples(add: &mut impl FnMut(&str, Value)) {
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
        }),
    );
    add(
        "skin.entry.uncredited",
        to(SkinEntry {
            id: format!("pkg:{}", pk('e')),
            name: "Plain".into(),
            imported_at: Some(1_760_000_000_001),
            credits: None,
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
}
