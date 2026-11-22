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
//! holds Rekindle's items for it.

use serde::{Deserialize, Serialize};

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
    Delete,
    DeleteSelected,
    OpenSelected,
    UploadSelected,
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

/// One `(event, action)` pair of a node, in the pack's order.
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NodeTrigger {
    pub on: SkinEvent,
    pub run: SkinAction,
}

/// One item of an engine menu.
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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MenuItem {
    pub label: String,
    pub run: SkinAction,
    /// A separator above the item.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub apart: bool,
    /// A destructive item, drawn as such.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub danger: bool,
}

/// One engine menu's items.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EngineMenuItems {
    pub menu: EngineMenu,
    pub items: Vec<MenuItem>,
}

/// Rekindle's items for each engine menu a pack opens.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MenuRegistry {
    pub menus: Vec<EngineMenuItems>,
}

fn item(label: &str, run: SkinAction) -> MenuItem {
    MenuItem {
        label: label.to_owned(),
        run,
        apart: false,
        danger: false,
    }
}

fn apart(mut i: MenuItem) -> MenuItem {
    i.apart = true;
    i
}

impl MenuRegistry {
    /// The engine menus with no Rekindle items yet: the chat window's
    /// (`CwActions`, `CwHelp`) and the group chat window's (`GcSystem`,
    /// `GcActions`, `GcHelp`) menus. They are planned feature gaps (design
    /// §2.2: built from the chat windows' existing actions); the pack's
    /// buttons for them stay.
    pub const FEATURE_GAPS: [EngineMenu; 5] = [
        EngineMenu::CwActions,
        EngineMenu::CwHelp,
        EngineMenu::GcSystem,
        EngineMenu::GcActions,
        EngineMenu::GcHelp,
    ];

    /// Rekindle's menus: File = the buddy list's first menu
    /// (`PackMenus.tsx` `first` + `MenuBar.tsx` Settings tabs and Quit),
    /// Tool = `tools` + `MenuBar.tsx` "Calls", Help = `help`, Status = the
    /// four presence choices, Sys = the window's system menu.
    #[must_use]
    pub fn rekindle() -> Self {
        use SkinAction as A;
        let file = vec![
            item("Add Friend…", A::AddFriend),
            item("New Chat…", A::NewChat),
            item("New Group…", A::NewGroup),
            apart(item("Create Community…", A::CreateCommunity)),
            item("Join Community…", A::JoinCommunity),
            apart(item("Profile", A::ShowSettings(SkinSettingsTab::Profile))),
            item("Privacy", A::ShowSettings(SkinSettingsTab::Privacy)),
            item("About", A::ShowSettings(SkinSettingsTab::About)),
            apart(item("Log Off", A::LogOff)),
            MenuItem {
                danger: true,
                ..item("Exit", A::Quit)
            },
        ];
        let tool = vec![
            item("Options", A::ShowSettings(SkinSettingsTab::Application)),
            item(
                "Notifications",
                A::ShowSettings(SkinSettingsTab::Notifications),
            ),
            item("Audio", A::ShowSettings(SkinSettingsTab::Audio)),
            item("Skins", A::ShowSettings(SkinSettingsTab::Appearance)),
            apart(item("Start Group Call…", A::HostGroupVoiceChat)),
            item("Keyboard Shortcuts", A::ShowShortcuts),
        ];
        let help = vec![
            item("Documentation", A::ShowView(EngineView::Help)),
            item("Report Bug", A::ReportBug),
        ];
        let status = vec![
            item("Online", A::SetStatus(SessionStatus::Online)),
            item("Away", A::SetStatus(SessionStatus::Away)),
            item("Busy", A::SetStatus(SessionStatus::Busy)),
            item("Appear Offline", A::SetStatus(SessionStatus::Invisible)),
        ];
        let sys = vec![
            item("Minimize", A::Minimize),
            item("Maximize", A::Maximize),
            apart(item("Close", A::Close)),
        ];
        let mut menus: Vec<EngineMenuItems> = [
            (EngineMenu::File, file),
            (EngineMenu::Tool, tool),
            (EngineMenu::Help, help),
            (EngineMenu::Status, status),
            (EngineMenu::Sys, sys),
        ]
        .into_iter()
        .map(|(menu, items)| EngineMenuItems { menu, items })
        .collect();
        menus.extend(Self::FEATURE_GAPS.map(|menu| EngineMenuItems {
            menu,
            items: Vec::new(),
        }));
        Self { menus }
    }

    /// `menu`'s items (empty for a [`Self::FEATURE_GAPS`] menu).
    #[must_use]
    pub fn items(&self, menu: EngineMenu) -> &[MenuItem] {
        self.menus
            .iter()
            .find(|m| m.menu == menu)
            .map_or(&[][..], |m| m.items.as_slice())
    }
}

#[cfg(test)]
mod tests {
    use super::{EngineMenu, MenuRegistry};

    /// Every engine menu has Rekindle items or is a recorded feature gap.
    #[test]
    fn every_engine_menu_is_populated_or_a_recorded_gap() {
        let reg = MenuRegistry::rekindle();
        for menu in EngineMenu::ALL {
            let gap = MenuRegistry::FEATURE_GAPS.contains(&menu);
            assert_eq!(reg.items(menu).is_empty(), gap, "{menu:?}");
            assert_eq!(reg.menus.iter().filter(|m| m.menu == menu).count(), 1);
        }
    }
}
