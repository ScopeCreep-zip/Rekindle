//! The closed role vocabulary of a skin tree's nodes (plan SK2, design
//! `sk2/design.md` §1): what the shell binds into each component an
//! Xfire pack names. A pack component maps to exactly one role; the
//! importers' name table lives in `rekindle-skin` (`skin_tree::roles`).

use serde::{Deserialize, Serialize};

/// The views of Xfire's sticky tab group (`TabbedControl`, Alt+1…7) and
/// Symbiosis's extra tabs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ViewKey {
    Friends,
    Servers,
    Files,
    ChatRooms,
    Pictures,
    Videos,
    Notifications,
}

/// The three menus of Xfire's menu bar (lang 228/229/231).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MenuKey {
    /// The first menu ("X&fire"; Rekindle's, plan SK2 lead decision 7).
    First,
    Tools,
    Help,
}

/// A button's action (design §3 N11/N16): the Friends, Servers, Files and
/// Chat Rooms rows' buttons. Combat's per-tab round buttons
/// (`AddDownloadTab`, `ChatRefreshStopTab`, `JoinPlayTab`,
/// `ProfileBrowseTab`) carry their Friends-tab action; the shell binds
/// the active view's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ActionKey {
    AddFriend,
    Chat,
    Join,
    Profile,
    AddServer,
    Refresh,
    StartDownload,
    StopDownload,
    OpenDownload,
    DownloadInfo,
    Browse,
    Find,
    EnterRoom,
    CreateRoom,
    DeleteItem,
    Favorite,
}

/// A resize edge or corner (Xfire's `Resize*` strips and `Gripper*` art).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Edge {
    Top,
    Bottom,
    Left,
    Right,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

/// An activity lamp (Combat's `DownloadBlink`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LampKey {
    /// A file transfer is running.
    Files,
}

/// What the shell binds into a node (design §1's 45 components, merged).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind", content = "key")]
pub enum NodeRole {
    /// The window's drag area (`MainTitleBar`, `WindowDragger`, `CombatDragBar`).
    Drag,
    /// The brand art (`XfireLogo`, `Logo`, `WndXfire`).
    Logo,
    /// The signed-in user's name (`UserNameField`, `Title`, `UsernameContainer`).
    NameField,
    Minimize,
    Maximize,
    Close,
    /// One menu's title.
    Menu(MenuKey),
    /// The menus' container (`MenuBar`, `MenuBackground`).
    MenuBar,
    /// The editable status text (`StatusEdit`, `Status`).
    StatusText,
    /// The status list's opener (`DownArrow`, `StatusBtn`).
    StatusList,
    /// The banner slot (`AdArea`, `AdBar`).
    AdArea,
    /// The tab group (`TabHeader`, Combat's knob `TabControl`).
    TabStrip,
    /// One tab.
    Tab(ViewKey),
    /// One stop of Combat's knob.
    Knob(ViewKey),
    /// The views' tab control (`TabbedControl` holding the buddy list).
    Views,
    /// One view (`BuddyView`, `ServerView`, `DownloadView`).
    View(ViewKey),
    /// The buddy list's column row (drawn by Xfire's `BuddyListView`).
    ColumnHeader,
    /// The per-tab button rows' tab control (`ButtonArea`).
    ButtonBar,
    /// One view's button row.
    ButtonRow(ViewKey),
    /// One button.
    Button(ActionKey),
    /// The InfoView toggle (`InfoViewButton`, `ExpandButton`, `InfoViewSwitch`).
    InfoToggle,
    /// The InfoView pane.
    InfoView,
    /// Symbiosis's chat-room name edit (`RoomEdit` in `ChatRoomsButtonBar`),
    /// bound to the group DM's name (plan SK2 lead decision 1).
    RoomName,
    /// StealthX's MENU rail (`MenuButton` → `MainMenuWindow`).
    SideRail,
    /// A resize edge or corner grip.
    Resize(Edge),
    Lamp(LampKey),
    /// Combat's console plate naming the active tab (`ConsoleText`).
    ConsoleLabel,
    /// A pack's own link (Total Chaos `TCLink`).
    PackLink,
    /// The current game's logo (Symbiosis `GameLogo`).
    GameLogo,
    /// Combat's window system menu.
    SystemMenu,
    /// A container with no behaviour of its own (`TopButtonBar`,
    /// `BaseBoard`, `BottomBar`, `StatusWrapper`).
    Group,
    /// A piece of its parent component: its state art or label
    /// (`FileMenu` under `XfireMenuText`, `TabNormalButton` under
    /// `TabNormalText`, `UserNameText` under `UserNameField`, a view's
    /// list ground); bound with the parent.
    Part,
    /// Pack art with no behaviour (frame pieces, `Vent`, `Divider`).
    Decor,
}

impl NodeRole {
    /// Whether the node takes input (validation rules 4 and 5).
    #[must_use]
    pub const fn interactive(self) -> bool {
        matches!(
            self,
            Self::NameField
                | Self::Minimize
                | Self::Maximize
                | Self::Close
                | Self::Menu(_)
                | Self::StatusText
                | Self::StatusList
                | Self::Tab(_)
                | Self::Knob(_)
                | Self::Button(_)
                | Self::InfoToggle
                | Self::SideRail
                | Self::RoomName
                | Self::Resize(_)
                | Self::PackLink
                | Self::SystemMenu
        )
    }

    /// Whether the role may appear more than once in a tree (rule 6).
    #[must_use]
    pub const fn repeats(self) -> bool {
        matches!(
            self,
            Self::Button(_)
                | Self::Decor
                | Self::Resize(_)
                | Self::Tab(_)
                | Self::Group
                | Self::Part
        )
    }
}
