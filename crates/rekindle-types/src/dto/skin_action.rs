//! A skin's behaviour (plan SK2-A, lead decision 36; design
//! `sk2a/design.md` §2.2): every Xfire pack wires its tiles with
//! `<Trigger Type="E">A</Trigger>` (XML) or `Action.<event>=A` /
//! `<state>.Trigger=<signal>` (INI). The names are the engine's own
//! (`Xfire.exe`'s UTF-16 action and event tables); this closed vocabulary
//! holds every one the 11 built-in packs and the census of 1,264 XML
//! triggers use, with typed arguments. The importer refuses any other name
//! (`ImportError::UnknownAction`); the Host maps each action to the
//! existing command through the ADR 0019 intent gate.
//!
//! Menu *items* were engine resources, not pack content (`v3/strings.xml`
//! `XfireMenu` 228, `ToolsMenu` 229, `HelpMenu` 231, `GCActionsMenu` 436):
//! a pack names which [`EngineMenu`] a tile opens, and [`MenuRegistry`]
//! holds Rekindle's items for it ([`super::skin_menu`], re-exported here).

use serde::{Deserialize, Serialize};

pub use super::skin_menu::{EngineMenuItems, MenuItem, MenuRegistry};
use super::skin_tree_role::Edge;
use crate::presence::SessionStatus;

/// One of the engine's menus (`File Tool Help Status GCSystem GCActions
/// GCHelp CWHelp CWActions`, plus the window's `Sys`/`System` menu).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EngineMenu {
    File,
    Tool,
    Help,
    Status,
    /// The window's system menu (`MenuSys`, `MenuSystem`, `ContextMenuSys`).
    Sys,
    CwActions,
    CwHelp,
    GcSystem,
    GcActions,
    GcHelp,
}

impl EngineMenu {
    /// Every engine menu, in the engine table's order.
    pub const ALL: [Self; 10] = [
        Self::File,
        Self::Tool,
        Self::Help,
        Self::Status,
        Self::Sys,
        Self::CwActions,
        Self::CwHelp,
        Self::GcSystem,
        Self::GcActions,
        Self::GcHelp,
    ];
}

/// A menu title's words: the names the packs' art bakes for the menu
/// (the engine's own `v3/strings.xml` `XfireMenu` 228 "Xfire", which
/// Alloy, Symbiosis and Total Chaos draw, and the plain "File" XGlow draws)
/// and the name Rekindle shows (plan SK2 lead decision 7: the first menu is
/// "Rekindle").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MenuTitleWords {
    /// What a pack's art may read for the menu.
    pub pack: &'static [&'static str],
    /// What the Host shows.
    pub ours: &'static str,
}

impl MenuTitleWords {
    /// Whether the Host renames the menu: a word a pack's art bakes is not
    /// Rekindle's, so the shell covers it and draws [`Self::ours`].
    #[must_use]
    pub fn renamed(self) -> bool {
        !self.pack.contains(&self.ours)
    }
}

impl EngineMenu {
    /// The one table of the menus' title words (pack names vs the Host's).
    #[must_use]
    pub const fn title(self) -> MenuTitleWords {
        let (pack, ours): (&'static [&'static str], &'static str) = match self {
            Self::File => (&["Xfire", "File"], "Rekindle"),
            Self::Tool => (&["Tools", "Tool"], "Tools"),
            Self::Help | Self::CwHelp | Self::GcHelp => (&["Help"], "Help"),
            Self::Status => (&["Status"], "Status"),
            Self::Sys | Self::GcSystem => (&["System"], "System"),
            Self::CwActions | Self::GcActions => (&["Actions"], "Actions"),
        };
        MenuTitleWords { pack, ours }
    }
}

/// A view an engine `Show<X>` action or event names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EngineView {
    Home,
    Friends,
    Chats,
    ChatRooms,
    Servers,
    Files,
    Videos,
    Screenshots,
    Notifications,
    Options,
    Help,
    InviteUsersToRoom,
    /// Rekindle's communities list (plan SK2-P1 rule N4).
    Communities,
}

/// A tile an action shows or hides: the engine's argument-less
/// `ShowTile`/`HideTile` act on the tile that carries the trigger.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind", content = "name")]
pub enum TileRef {
    This,
    Named(String),
}

/// The popup a `LaunchSkin` opens: the component's `<Popup>` (XML) or its
/// `Skin=` file (INI).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PopupRef {
    pub popup: String,
}

/// `Animate(Type=Repeat,Delay=4,Repeat=-1,Action=1)`: repeat a state every
/// `delay` ticks, `repeat` times (`None` = forever, the pack's `-1`),
/// `action` naming the state set it blinks to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnimateSpec {
    pub delay: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repeat: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<u16>,
}

/// The state set a `State(...)` trigger selects (spec A10).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NodeState {
    Normal,
    Disabled,
    Down,
}

/// A pack link's URL (`AppLaunchUrl(http://www.totalchaos.org.uk/)`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PackUrl {
    pub url: String,
}

/// A pack sound (`PlaySound(OfflineSound)`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SoundRef {
    pub sound: String,
}

/// An in-game overlay part the XIG actions and preference events name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OverlayPart {
    Fps,
    Clock,
    Alert,
    WebCam,
    VoiceOverlay,
    Browser,
}

/// A Settings tab a Rekindle menu item opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SkinSettingsTab {
    Profile,
    Privacy,
    About,
    Application,
    Notifications,
    Audio,
    Appearance,
}

/// Every engine action, closed (design §2.1 census + §2.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind", content = "arg")]
pub enum SkinAction {
    // window
    DragWindow,
    Minimize,
    Maximize,
    Close,
    Resize(Edge),
    // menus
    Menu(EngineMenu),
    ContextMenu(EngineMenu),
    // views and tiles
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
    /// INI `AnimationStart(GroupId=100,Type=Repeat,Delay=7,Repeat=-1)`:
    /// animate every tile of control group `group` (Combat's download lamp).
    AnimationStart {
        group: u16,
        spec: AnimateSpec,
    },
    /// INI `AnimationStop(GroupId=100)`.
    AnimationStop {
        group: u16,
    },
    SetState(NodeState),
    // friends and chat
    AddFriend,
    ChatFriend,
    ProfileFriend,
    ViewProfile,
    JoinFriend,
    SendMessage,
    // voice
    VoiceChat,
    HostVoiceChat,
    LeaveVoiceChat,
    GroupVoiceChat,
    HostGroupVoiceChat,
    LeaveGroupVoiceChat,
    CancelVoiceChat,
    EndVoiceChat,
    MuteSelf,
    UnmuteSelf,
    ShowVoiceWindow,
    // game lobbies and servers
    HostGameLobby,
    JoinGameLobby,
    LeaveGameLobby,
    LaunchGameLobby,
    AddServer,
    RefreshServer,
    // downloads
    StartDownload,
    StopDownload,
    OpenDownload,
    DownloadInfo,
    // rooms and files
    NewRoom,
    CreateRoom,
    EnterRoom,
    EnterAttachedRoom,
    JoinRoom,
    InviteUsersToRoom,
    DeleteSelected,
    OpenSelected,
    UploadSelected,
    SearchFor,
    LaunchUrl(PackUrl),
    PlaySound(SoundRef),
    // in-game overlay (Phase 6 windows)
    ShowWindow,
    HideWindow,
    DialogOk,
    DialogCancel,
    EnableOverlay(OverlayPart),
    DisableOverlay(OverlayPart),
    UpdateClock,
    TackWindow,
    UntackWindow,
    OpenOverlayOptions,
    AttachedClose,
    BrowseBack,
    BrowseForward,
    // Rekindle extensions (the default pack, §3 placements and menus)
    NewChat,
    NewGroup,
    CreateCommunity,
    JoinCommunity,
    ShowSettings(SkinSettingsTab),
    ShowShortcuts,
    ReportBug,
    SetStatus(SessionStatus),
    LogOff,
    Quit,
    CopyKey,
    EditName,
    ShowSafetyNumber,
    ShowGameInvite,
    VideoChat,
    OpenNotifications,
    /// The Communities view's row (plan E7.7): open the selected
    /// community's window.
    OpenCommunity,
    /// Leave the selected community (the shell's leave, through the
    /// Host's leave intent).
    LeaveCommunity,
    /// The Create or Join chooser.
    StartCommunity,
}

/// A capability of the selected item (`OnAppCan<X>` / `OnAppCannot<X>`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Capability {
    AddFriend,
    Chat,
    Profile,
    Join,
    VoiceChat,
    HostVoiceChat,
    LeaveVoiceChat,
    CancelVoiceChat,
    MuteSelf,
    UnmuteSelf,
    AddServer,
    RefreshServer,
    StartDownload,
    StopDownload,
    ExecuteDownload,
    HostGameLobby,
    JoinGameLobby,
    LaunchGameLobby,
    LeaveGameLobby,
    CreateRoom,
    EnterRoom,
    /// Rename the selected room: we created it (lead decision 1a; the
    /// `RoomName` field's edit). Rekindle's own: Xfire's rooms were
    /// renamed on the server.
    RenameRoom,
    Delete,
    DeleteSelected,
    OpenSelected,
    UploadSelected,
    /// A community is selected in the Communities view (plan E7.7).
    OpenCommunity,
    LeaveCommunity,
}

/// Backend-owned state an `On<X>` trigger reacts to (design §2.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind", content = "arg")]
pub enum AppSignal {
    Can(Capability),
    Cannot(Capability),
    ActiveUserTyping,
    ActiveUserNotTyping,
    VoiceRinging,
    UserStartedTalking,
    UserStoppedTalking,
    IncomingVoiceChat,
    OutgoingVoiceChat,
    StartVoiceChat,
    EndVoiceChat,
    SpeakerOn,
    SpeakerOff,
    PushToTalkOn,
    PushToTalkOff,
    NewNotification,
    NotificationsVisited,
    DownloadComplete,
    XfireUrlDownload,
    Disconnected,
    StartingLogin,
    StartRefreshServers,
    StopRefreshServers,
    PrefEnabled(OverlayPart),
    PrefDisabled(OverlayPart),
    GamePrefBrowserDisabled,
    WebCamOverlayEnabled,
    WebCamOverlayDisabled,
}

/// What fires a trigger: a pointer or window event, a view switch, or an
/// [`AppSignal`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind", content = "arg")]
pub enum SkinEvent {
    Click,
    DoubleClick,
    LeftDown,
    RightDown,
    RightClick,
    Activate,
    Deactivate,
    ActiveMode,
    InactiveMode,
    TabActivated,
    CheckboxChecked,
    CheckboxUnchecked,
    Tacked,
    Untacked,
    FilesFocus,
    FilesKillFocus,
    Show(EngineView),
    App(AppSignal),
}

impl SkinEvent {
    /// Whether the event is a pointer press on the node (click, double
    /// click, button down, right click): what makes a node interactive.
    #[must_use]
    pub const fn pointer(&self) -> bool {
        matches!(
            self,
            Self::Click | Self::DoubleClick | Self::LeftDown | Self::RightDown | Self::RightClick
        )
    }
}

/// One `(event, action)` pair of a node, in the pack's order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NodeTrigger {
    pub on: SkinEvent,
    pub run: SkinAction,
}

impl NodeTrigger {
    /// Whether the trigger makes its node a control: a primary press
    /// (click or button down) that runs anything but the window drag. The
    /// frame's ground keeps its own gestures without becoming a control:
    /// `AppDragWindow` on button down, Onyx's `TopLeft` double-click
    /// `AppMaximize` (`components.xml:183-190`), XGlow's `[Bottom]`
    /// right-click `MenuTool` (`Xglow.ini:494-498`).
    #[must_use]
    pub fn presses(&self) -> bool {
        matches!(self.on, SkinEvent::Click | SkinEvent::LeftDown)
            && self.run != SkinAction::DragWindow
    }

    /// Whether the trigger is a pointer gesture on the window's frame, as
    /// a title bar takes it in every OS (Tauri's `data-tauri-drag-region`):
    /// a drag that moves the window, a double click that maximizes it, or
    /// a drag that resizes it. A click or button down that maximizes is a
    /// maximize button, not a gesture.
    #[must_use]
    pub fn frame_gesture(&self) -> bool {
        self.on.pointer()
            && match self.run {
                SkinAction::DragWindow | SkinAction::Resize(_) => true,
                SkinAction::Maximize => self.on == SkinEvent::DoubleClick,
                _ => false,
            }
    }
}
