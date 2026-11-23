//! What a skin action does (plan SK2-A4, design §2.2): the Host maps each
//! [`SkinAction`] a pack's trigger names to an `IpcRequest` run through
//! the intent gate (ADR 0019), to a [`ShellOp`] the window's shell runs
//! (the OS drag/resize must start inside the pointer event; a panel or a
//! menu opens in the window), or to [`SkinRun::NotBuiltYet`] for an
//! action Rekindle has no feature for yet, which the window shows (never
//! silently nothing). The backend owns the selection a selection-dependent
//! action acts on ([`SkinSelection`]) and the per-window signals the
//! pack's state triggers follow ([`SkinSignals`], design §2.5).

use serde::{Deserialize, Serialize};

use super::skin_action::{
    AnimateSpec, AppSignal, EngineMenu, EngineView, NodeState, PopupRef, SkinAction,
    SkinSettingsTab, SoundRef, TileRef,
};
use super::skin_layout::SkinWindow;
use super::skin_tree::Edge;

/// A panel of the window's shell an action opens (today's dialogs).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ShellPanel {
    AddFriend,
    NewChat,
    NewGroup,
    StartGroupCall,
    CreateCommunity,
    JoinCommunity,
    Shortcuts,
    Notifications,
}

/// A 1:1 call operation: the shell's call commands own calls until plan
/// E4.1 moves them into the Host (then these rows become requests).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind", content = "arg")]
pub enum CallOp {
    Voice { friend: String },
    Video { friend: String },
    End,
    Mute,
    Unmute,
}

/// An operation the window's shell runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind", content = "arg")]
pub enum ShellOp {
    DragWindow,
    Minimize,
    Maximize,
    Close,
    Resize(Edge),
    /// An engine menu, its items from `MenuRegistry`; `context` for a
    /// pointer-placed one (`ContextMenu`).
    OpenMenu {
        menu: EngineMenu,
        context: bool,
    },
    ShowView(EngineView),
    GroupPage {
        group: u16,
        page: u16,
    },
    ShowTile(TileRef),
    HideTile(TileRef),
    LaunchPopup(PopupRef),
    Animate(AnimateSpec),
    AnimateStop,
    AnimationStart {
        group: u16,
        spec: AnimateSpec,
    },
    AnimationStop {
        group: u16,
    },
    SetState(NodeState),
    PlaySound(SoundRef),
    OpenPanel(ShellPanel),
    OpenChat {
        friend: String,
    },
    OpenProfile {
        friend: String,
    },
    OpenSafetyNumber {
        friend: String,
    },
    OpenGameInvite {
        friend: String,
    },
    /// The New Group panel, prefilled with `peer` when the window has a
    /// friend (SK2-G §6: `NewRoom`/`CreateRoom`); the group it creates
    /// records `peer` as the chat it came from.
    NewRoom {
        peer: Option<String>,
    },
    /// The group window of `group_id` (`EnterRoom`, `JoinRoom`,
    /// `EnterAttachedRoom` on a joined group).
    OpenGroupChat {
        group_id: String,
    },
    /// The friend picker that invites more into the joined group
    /// `group_id` (`InviteUsersToRoom`, SK2-G §6, plan E7.2 G8): its
    /// choice is one `GroupDmInvite`, and the group's `inviteRefused`
    /// verdict says when the Host refuses it.
    InviteToRoom {
        group_id: String,
    },
    Call(CallOp),
    ShowSettings(SkinSettingsTab),
    /// Ask the user where to save a never-fetched attachment (the native
    /// save dialog, `suggested_name` the uploader's sanitized filename),
    /// then send `TransferStart` with that destination (plan E7.2 G12).
    ChooseDownloadDest {
        attachment_id: String,
        target: super::transfer::TransferTarget,
        suggested_name: String,
    },
    /// The local search panel, scoped to the view it was opened from
    /// (`SearchFor`, SK2-G §8); its query is one `LocalSearch`.
    OpenSearch {
        scope: super::local_search::LocalSearchScope,
    },
    ReportBug,
    CopyKey,
    EditName,
    Quit,
}

/// The answer to `RunSkinAction` when the action is not a request (a
/// request's answer is that request's own).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind", content = "arg")]
pub enum SkinRun {
    Shell(ShellOp),
    /// Rekindle has no feature for `action` yet (design §2.2 "Feature
    /// gaps"): the button stays, and the window says so.
    NotBuiltYet {
        action: SkinAction,
    },
    /// The operation is the window labelled `window`'s, which the Host
    /// handed it on that window's channel (`SystemEvent::SkinShell`): a
    /// view switch pressed in a popup runs in its launcher (plan SK2-N4).
    /// The window pressed runs nothing.
    Launcher {
        window: String,
    },
}

/// What a window has selected, kept by the Host (design §2.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind",
    content = "arg"
)]
pub enum SkinSelection {
    Friend {
        public_key: String,
    },
    /// A game server in the Servers view (SK2-G §3): its game and `host:port`.
    Server {
        game_id: u32,
        address: String,
    },
    /// A download in the transfers view, or an offered attachment in a
    /// channel (SK2-G §4), by attachment id and where it lives.
    Transfer {
        attachment_id: String,
        target: super::transfer::TransferTarget,
    },
    /// A picture or video in the media view (SK2-G §7), by its file.
    Media {
        path: String,
    },
    /// A group DM, a room (SK2-G §5–§6), by group id.
    Group {
        group_id: String,
    },
    /// A friend group's header in the buddy list (`buddycat` /
    /// `customgroup`, plan E7.15 IV5): its Infoview page. `None` is the
    /// default group of friends in no group.
    FriendGroup {
        group_id: Option<i64>,
    },
}

/// A window's current signal set (design §2.5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkinSignals {
    pub window: SkinWindow,
    pub set: Vec<AppSignal>,
}
