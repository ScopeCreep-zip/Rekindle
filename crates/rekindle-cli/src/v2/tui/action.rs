//! Action types for the TUI dispatch loop.
//!
//! All user intents and async results flow through [`Action`].

/// Unified action type for all TUI state transitions.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum Action {
    // ── Lifecycle ──────────────────────────────────────────
    Tick,
    Render,
    Quit,
    Resize(u16, u16),

    // ── Navigation ────────────────────────────────────────
    FocusNext,
    FocusPrev,
    ScrollUp(u16),
    ScrollDown(u16),
    ScrollPageUp,
    ScrollPageDown,
    ScrollToTop,
    ScrollToBottom,
    Select,
    Cancel,
    Back,
    NextTab,
    PrevTab,

    // ── Global ─────────────────────────────────────────────
    ToggleHelp,
    Refresh,
    OpenSearch(SearchMode),
    OpenQuickSwitcher,
    OpenFileContentSearch,
    ToggleSidebar,

    // ── Input ──────────────────────────────────────────────
    EnterInputMode,
    ExitInputMode,
    InputSubmit,
    ReplyToSelected,
    EditSelected,

    // ── View transitions ───────────────────────────────────
    ShowDashboard,
    ShowIdentitySettings,
    ShowChannel { community: String, channel: String },
    ShowDmInbox,
    ShowDmThread { peer_key: String },
    ShowFriendList,
    ShowVoiceSession { community: String, channel: String },
    ShowDoctor,
    ShowCommunityInfo { community: String },
    ShowFilePreview { path: String, line: Option<usize> },

    // ── Split pane DM (channel watch right side) ───────────
    OpenSplitDm { peer_key: String },
    CloseSplitDm,

    // ── Message operations ─────────────────────────────────
    SendChannelMessage { community: String, channel: String, text: String, reply_to: Option<String> },
    SendDm { peer_key: String, text: String },
    EditMessage { community: String, channel: String, message_id: String, new_body: String },
    DeleteMessage { community: String, channel: String, message_id: String },
    SendChannelTyping { community: String, channel: String },
    SendDmTyping { peer_key: String },

    /// Scroll the active message list to a specific message by ID.
    ScrollToMessage { message_id: String },

    // ── Patch operations ──────────────────────────────────
    /// Apply a patch from a message's ```patch fence to the local working tree.
    ApplyPatch { message_id: String },
    /// Copy a patch's raw diff text to clipboard.
    CopyPatch { message_id: String },
    /// Collapse/dismiss a patch in the message list (toggle).
    DismissPatch { message_id: String },

    // ── Voice ──────────────────────────────────────────────
    JoinVoice { community: String, channel: String },
    LeaveVoice,
    ToggleMute,
    ToggleDeafen,

    // ── Friend operations ──────────────────────────────────
    AcceptFriendRequest(String),
    RejectFriendRequest(String),
    RemoveFriend { peer_key: String },

    // ── File selection ──────────────────────────────────────
    /// A file was selected from the quick switcher. If input mode is active,
    /// insert as backtick-wrapped inline code reference. Otherwise, open
    /// file preview view.
    FileSelected { path: String },

    // ── Clipboard ───────────────────────────────────────────
    YankToClipboard { text: String },

    // ── Community operations ───────────────────────────────
    LeaveCommunity { community: String },

    // ── Key operations ─────────────────────────────────────
    RequestMek { community: String, channel: String },

    // ── Presence ───────────────────────────────────────────
    SetPresence { status: String, message: Option<String> },

    // ── Async results ──────────────────────────────────────
    CommandComplete(Box<CommandResult>),
    CommandFailed { context: String, error: String },

    // ── Overlay ────────────────────────────────────────────
    OpenOverlay(OverlayKind),
    CloseOverlay,
    ConfirmOverlay,

    // ── Notifications ──────────────────────────────────────
    ShowToast { message: String, level: ToastLevel },
    DismissToast,

    // ── Subscription events ────────────────────────────────
    /// Boxed to avoid inflating the Action enum (SubscriptionEvent is ~256 bytes).
    SubscriptionEvent(Box<rekindle_types::subscription_events::SubscriptionEvent>),
}

/// Search overlay mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchMode {
    QuickSwitch,
    MessageSearch,
    CommandPalette,
}

/// Modal overlay type.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum OverlayKind {
    Help,
    Search(SearchMode),
    ConfirmAction { prompt: String, consequence: String, action: Box<Action> },
}

/// Toast severity level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastLevel {
    Info,
    Success,
    Warning,
    Error,
}

/// Async command results using display types from `rekindle-types`.
#[derive(Debug, Clone)]
pub enum CommandResult {
    MessageSent { message_id: String },
    ChannelHistoryLoaded {
        community: String,
        channel: String,
        messages: Vec<rekindle_types::display::DecryptedMessageDisplay>,
    },
    CommunityListLoaded { communities: Vec<rekindle_types::display::CommunityOverview> },
    CommunityInfoLoaded { detail: rekindle_types::display::CommunityDetail },
    FriendListLoaded { friends: Vec<rekindle_types::display::FriendDisplay> },
    DmInboxLoaded { threads: Vec<rekindle_types::display::DmThreadDisplay> },
    DmThreadLoaded { peer_key: String, messages: Vec<rekindle_types::display::DmMessageDisplay> },
    StatusLoaded { snapshot: rekindle_types::display::StatusSnapshot },
    PeerListLoaded { peers: Vec<rekindle_types::display::PeerSnapshot> },
    IdentityLoaded {
        public_key: String,
        display_name: String,
        profile_dht_key: String,
        mailbox_dht_key: String,
        friend_list_dht_key: String,
        friend_inbox_key: String,
    },
    SendFailed,
}
