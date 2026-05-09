//! Action types for the TUI dispatch loop.
//!
//! All user intents and async results flow through [`Action`].
//! Components emit actions via `handle_key`. The App processes them
//! in `process_action`. Display types from `rekindle_types::display`
//! are used directly in [`CommandResult`].

/// Unified action type for all TUI state transitions.
#[derive(Debug, Clone)]
#[allow(dead_code)] // M3 wires remaining variants
pub enum Action {
    // ─── Lifecycle ──────────────────────────────────────────────
    Tick,
    Render,
    Quit,
    Resize(u16, u16),

    // ─── Navigation ────────────────────────────────────────────
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

    // ─── Global ─────────────────────────────────────────────────
    ToggleHelp,
    Refresh,
    OpenSearch(SearchMode),
    OpenQuickSwitcher,
    ToggleSidebar,

    // ─── Input ──────────────────────────────────────────────────
    EnterInputMode,
    ExitInputMode,
    InputSubmit,
    /// Reply to the currently selected message.
    ReplyToSelected,
    /// Edit the currently selected message (own messages only).
    EditSelected,

    // ─── View transitions ───────────────────────────────────────
    ShowDashboard,
    ShowIdentitySettings,
    ShowChannel { community: String, channel: String },
    ShowDmInbox,
    ShowDmThread { peer_key: String },
    ShowFriendList,
    ShowVoiceSession { community: String, channel: String },
    ShowDoctor,
    ShowCommunityInfo { community: String },

    // ─── Message operations ─────────────────────────────────────
    SendChannelMessage {
        community: String,
        channel: String,
        text: String,
        reply_to: Option<String>,
    },
    SendDm { peer_key: String, text: String },
    EditMessage { community: String, channel: String, message_id: String, new_body: String },
    DeleteMessage { community: String, channel: String, message_id: String },

    // ─── Voice ──────────────────────────────────────────────────
    JoinVoice { community: String, channel: String },
    LeaveVoice,
    ToggleMute,
    ToggleDeafen,

    // ─── Friend operations ──────────────────────────────────────
    AcceptFriendRequest(String),
    RejectFriendRequest(String),
    /// Remove a friend — destructive, requires confirmation.
    RemoveFriend { peer_key: String },

    // ─── Clipboard ───────────────────────────────────────────────
    /// Copy focused message body to clipboard with 30s auto-clear.
    YankToClipboard { text: String },

    // ─── Community operations ───────────────────────────────────
    /// Leave a community — destructive, requires confirmation.
    LeaveCommunity { community: String },

    // ─── Key operations ─────────────────────────────────────────
    RequestMek { community: String, channel: String },

    // ─── Presence ───────────────────────────────────────────────
    SetPresence { status: String, message: Option<String> },

    // ─── Async results ──────────────────────────────────────────
    CommandComplete(Box<CommandResult>),
    CommandFailed { context: String, error: String },

    // ─── Overlay ────────────────────────────────────────────────
    OpenOverlay(OverlayKind),
    CloseOverlay,
    ConfirmOverlay,

    // ─── Notifications ──────────────────────────────────────────
    ShowToast { message: String, level: ToastLevel },
    DismissToast,

    // ─── Real-time subscription events ─────────────────────────
    /// Subscription event from the daemon's three-tier pipeline.
    /// The reducer dispatches to the active view for view-specific state mutation.
    /// Boxed to avoid inflating the entire Action enum (SubscriptionEvent is ~256 bytes).
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
#[allow(dead_code)] // M3 wires Search and ConfirmAction
pub enum OverlayKind {
    Help,
    Search(SearchMode),
    ConfirmAction {
        prompt: String,
        consequence: String,
        action: Box<Action>,
    },
}

/// Toast severity level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastLevel {
    Info,
    Success,
    Warning,
    Error,
}

/// Async command results using shared display types from `rekindle-types`.
#[derive(Debug, Clone)]
pub enum CommandResult {
    MessageSent {
        message_id: String,
    },
    ChannelHistoryLoaded {
        community: String,
        channel: String,
        messages: Vec<rekindle_types::display::DecryptedMessageDisplay>,
    },
    CommunityListLoaded {
        communities: Vec<rekindle_types::display::CommunityOverview>,
    },
    CommunityInfoLoaded {
        detail: rekindle_types::display::CommunityDetail,
    },
    FriendListLoaded {
        friends: Vec<rekindle_types::display::FriendDisplay>,
    },
    DmInboxLoaded {
        threads: Vec<rekindle_types::display::DmThreadDisplay>,
    },
    DmThreadLoaded {
        peer_key: String,
        messages: Vec<rekindle_types::display::DmMessageDisplay>,
    },
    StatusLoaded {
        snapshot: rekindle_types::display::StatusSnapshot,
    },
    PeerListLoaded {
        peers: Vec<rekindle_types::display::PeerSnapshot>,
    },
    IdentityLoaded {
        public_key: String,
        display_name: String,
    },
    /// A message send (channel or DM) failed after the view already
    /// inserted a pending message with `DeliveryStatus::Sending`.
    /// The view flips the pending message to `DeliveryStatus::Failed` (✗).
    SendFailed,
}
