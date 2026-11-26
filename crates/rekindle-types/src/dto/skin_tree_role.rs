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

impl ViewKey {
    /// The Host's name for the view (plan SK6), the words the shell draws on
    /// a tab whose pack draws its word as text (Classic's `TabNormalText`,
    /// `Class="Text"`, whose `FriendsTitle` string is a resource of the Xfire
    /// binary, `strings.xml:21`): the app's own tab bar's labels
    /// (`src/components/buddy-list/TabBar.tsx:6-11`), which keep Xfire's
    /// "Downloads" for the files view.
    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            Self::Friends => "Friends",
            Self::Servers => "Servers",
            Self::Files => "Downloads",
            Self::ChatRooms => "Chat Rooms",
            Self::Pictures => "Pictures",
            Self::Videos => "Videos",
            Self::Notifications => "Notifications",
            Self::Communities => "Communities",
        }
    }
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

impl Edge {
    /// Whether the edge is a corner (it resizes on both axes).
    #[must_use]
    pub const fn corner(self) -> bool {
        matches!(
            self,
            Self::TopLeft | Self::TopRight | Self::BottomLeft | Self::BottomRight
        )
    }
}

/// An activity lamp (Combat's `DownloadBlink`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LampKey {
    /// A file transfer is running.
    Files,
}

/// A box in the chat window's tree that the shell mounts its own control
/// into (plan SK17-2): the pack places it, Rekindle fills it, as a
/// [`NodeRole::View`] is filled with the buddy list's views.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SlotKey {
    /// The "is typing" line (Onyx's and Space Cowboy's `TypingMessageBG`,
    /// Symbiosis's `ChatTypingMessage`, xSLIMFIRE's `TypingMessage`).
    TypingNotice,
    /// The conversation's message list (every pack's `ChatArea`).
    ChatArea,
    /// The composer (every pack's `ChatEdit`).
    ChatEdit,
    /// The send button (Symbiosis's `ChatButtonSend`, xSLIMFIRE's
    /// `SendButton` in its `ChatBaseBoard`, Onyx's and Space Cowboy's
    /// `ChatImButton`).
    ChatSend,
    /// The InfoView toggle (xSLIMFIRE's `ChatInfoViewButton`, Space
    /// Cowboy's `ChatExpandButton`, Symbiosis's `ButtonInfo`).
    InfoviewButton,
}

/// Which window-frame button a chat window's [`NodeRole::WindowButton`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WindowButtonKind {
    Minimize,
    Maximize,
    Close,
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
    /// The chat window's title strip (Onyx's and Space Cowboy's
    /// `MainTitleBar`, xSLIMFIRE's `ChatTitleBar`, Symbiosis's `InfoBar`),
    /// a window drag area like [`Self::Drag`] (plan SK17-2).
    TitleBar,
    /// The chat window's conversation title. The tree carries the pack's
    /// font and colour, never words: the shell writes the conversation's
    /// title the Host serves that window (the peer's display name, the
    /// group's name), never the pack's own `ChatTitle`/`UserNameText`.
    ChatTitle,
    /// A window-frame button, with the pack's art states; absent where
    /// the pack draws none (xSLIMFIRE has no maximize).
    WindowButton(WindowButtonKind),
    /// Symbiosis's `TabbedTalkingState`: the peer's typing lamp, its two
    /// parts shown by the window's `userTyping` signal.
    TalkingState,
    /// A box the shell mounts a Rekindle control into ([`SlotKey`]).
    Slot(SlotKey),
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
