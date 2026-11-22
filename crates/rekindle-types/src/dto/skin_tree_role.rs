//! The closed role vocabulary of a skin tree's nodes (plan SK2, design
//! `sk2/design.md` §1): what the shell binds into each component an
//! Xfire pack names. A pack component maps to exactly one role; the
//! importers' name table lives in `rekindle-skin` (`skin_tree::roles`).

use serde::{Deserialize, Serialize};

use super::skin_action::NodeTrigger;

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
    /// Rekindle's communities list (plan SK2-P1 rule N4): the Host places
    /// it in every main-window tree beside `Friends`.
    Communities,
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
    /// One menu's title (`XfireMenuText`, `ToolsMenu`): the menu it opens
    /// is its `Menu<Name>` trigger's (plan SK2 decision 36).
    MenuTitle,
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
    /// A pressable control: what it does is its triggers (plan SK2
    /// decision 36: the window boxes, buttons, the InfoView toggle, the
    /// side rail's toggle, a pack's link, the system menu).
    Button,
    /// The InfoView pane.
    InfoView,
    /// Symbiosis's chat-room name edit (`RoomEdit` in `ChatRoomsButtonBar`),
    /// bound to the group DM's name (plan SK2 lead decision 1).
    RoomName,
    /// StealthX's MENU rail: the launched `MainMenuWindow` popup
    /// (`popups.xml:2-89`) holding the menus, tabs and button rows.
    SideRail,
    Lamp(LampKey),
    /// Combat's console plate naming the active tab (`ConsoleText`).
    ConsoleLabel,
    /// The current game's logo (Symbiosis `GameLogo`).
    GameLogo,
    /// A container with no behaviour of its own (`TopButtonBar`,
    /// `BaseBoard`, `BottomBar`, `StatusWrapper`).
    Group,
    /// A piece of its parent component (a resize grip's strip, its state
    /// art or label
    /// (`FileMenu` under `XfireMenuText`, `TabNormalButton` under
    /// `TabNormalText`, `UserNameText` under `UserNameField`, a view's
    /// list ground); bound with the parent.
    Part,
    /// Pack art with no behaviour (frame pieces, `Vent`, `Divider`).
    Decor,
}

impl NodeRole {
    /// Whether the role takes input whatever its triggers: the editable
    /// name, status and room fields, the status list and the tabs.
    #[must_use]
    pub const fn input(self) -> bool {
        matches!(
            self,
            Self::NameField
                | Self::StatusText
                | Self::StatusList
                | Self::Tab(_)
                | Self::Knob(_)
                | Self::RoomName
        )
    }

    /// Whether a node of this role with `triggers` takes input
    /// (validation rules 4 and 5; design §2.4): it is an input role or a
    /// trigger presses it ([`NodeTrigger::presses`]). A part never does: it
    /// is a piece of its parent control, whose triggers it carries.
    #[must_use]
    pub fn interactive(self, triggers: &[NodeTrigger]) -> bool {
        !matches!(self, Self::Part) && (self.input() || triggers.iter().any(NodeTrigger::presses))
    }

    /// Whether a tree holds at most one node of the role (rule 6, plan SK2
    /// lead decision 7): the title strip, the name and status fields and
    /// the views; every other role may repeat per view or tab (Symbiosis's
    /// two button bars).
    #[must_use]
    pub const fn singleton(self) -> bool {
        matches!(
            self,
            Self::Drag | Self::NameField | Self::StatusText | Self::Views
        )
    }
}
