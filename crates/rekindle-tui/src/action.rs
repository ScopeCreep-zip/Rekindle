//! Action types for the TUI dispatch loop.
//!
//! All user intents and async results flow through [`Action`].
//! Components emit actions via `handle_key`. The App processes them
//! in `process_action`. Display types from `rekindle_types::display`
//! are used directly in [`CommandResult`].

/// Unified action type for all TUI state transitions.
#[derive(Debug, Clone)]
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
    ShowChannel {
        community: String,
        channel: String,
    },
    ShowDmInbox,
    ShowDmThread {
        peer_key: String,
    },
    ShowFriendList,
    ShowVoiceSession {
        community: String,
        channel: String,
    },
    ShowDoctor,
    ShowCommunityInfo {
        community: String,
    },

    // ─── Message operations ─────────────────────────────────────
    SendChannelMessage {
        community: String,
        channel: String,
        text: String,
        reply_to: Option<String>,
    },
    SendDm {
        peer_key: String,
        text: String,
    },

    // ─── Voice ──────────────────────────────────────────────────
    LeaveVoice,
    ToggleMute,
    ToggleDeafen,

    // ─── Friend operations ──────────────────────────────────────
    /// Accept a pending request (the Host names them from it).
    AcceptFriendRequest {
        public_key: String,
    },
    RejectFriendRequest(String),
    /// Remove a friend — destructive, requires confirmation.
    RemoveFriend {
        peer_key: String,
    },
    /// Block a friend or requester (the host asks to confirm).
    BlockUser {
        public_key: String,
    },
    UnblockUser {
        public_key: String,
    },
    /// Cancel one of our open friend invites.
    CancelInvite {
        invite_id: String,
    },

    // ─── Clipboard ───────────────────────────────────────────────
    /// Copy focused message body to clipboard with 30s auto-clear.
    YankToClipboard {
        text: String,
    },

    // ─── Community operations ───────────────────────────────────
    /// Leave a community — destructive, requires confirmation.
    LeaveCommunity {
        community: String,
    },

    // ─── Async results ──────────────────────────────────────────
    CommandComplete(Box<CommandResult>),
    CommandFailed {
        context: String,
        error: String,
    },

    // ─── Overlay ────────────────────────────────────────────────
    CloseOverlay,

    // ─── Intent reviews (plan D3.4) ─────────────────────────────
    /// The host holds an action for review: show it.
    ShowIntent(Box<rekindle_types::intent::IntentReview>),
    /// Our own presence changed (the host's `SelfChanged`).
    OwnPresence(Box<rekindle_types::subscription_events::PresenceSnapshot>),
    /// Commit the reviewed action with this proof.
    CommitIntent {
        review: Box<rekindle_types::intent::IntentReview>,
        proof: rekindle_types::intent::IntentProof,
    },
    /// Drop the reviewed action.
    CancelIntent {
        intent_id: rekindle_types::intent::IntentId,
    },

    // ─── Notifications ──────────────────────────────────────────
    ShowToast {
        message: String,
        level: ToastLevel,
    },

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
///
/// Search and intent reviews are handled by their own dedicated state
/// (`SearchOverlay`, `IntentOverlay`); this enum covers the simple
/// full-screen help overlay.
#[derive(Debug, Clone)]
pub enum OverlayKind {
    Help,
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
        friends: Vec<rekindle_types::dto::friends::Friend>,
    },
    /// The requests waiting for our answer, the people we blocked, and our
    /// open invites (plan E2.1).
    FriendExtrasLoaded {
        requests: Vec<rekindle_types::dto::friends::PendingRequest>,
        blocked: Vec<rekindle_types::dto::friends::BlockedUser>,
        invites: Vec<rekindle_types::dto::friends::OutgoingInvite>,
    },
    DmInboxLoaded {
        threads: Vec<rekindle_types::display::DmThreadDisplay>,
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
    /// Our friend code and its QR (`FriendCode`, plan E2.5).
    FriendCodeLoaded {
        view: rekindle_types::dto::friends::FriendCodeView,
    },
    /// Our own status, message, game and idle state (`PresenceGet`).
    OwnPresenceLoaded {
        view: rekindle_types::dto::presence::OwnPresenceView,
    },
}
