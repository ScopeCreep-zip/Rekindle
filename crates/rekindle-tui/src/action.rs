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
    /// Delete the selected message: ours for members, another member's as
    /// a moderator's removal (plan E3.3; the Host judges either).
    DeleteSelected,

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
    /// Our group DMs and pending invites (plan E2.6).
    ShowGroupDms,
    /// Load a group's stored messages into the group view.
    ShowGroupMessages {
        group_id: String,
    },
    /// Load a DM thread's stored messages (`DmHistory`), with their
    /// delivery states, into the open DM view (plan SR2).
    LoadDmHistory {
        peer_key: String,
    },
    ShowVoiceSession {
        community: String,
        channel: String,
    },
    ShowDoctor,
    /// A friend's profile (`peer_key`), or ours (`None`): bio and game
    /// time (plan E7.2). `name` is what the opener showed.
    ShowProfile {
        peer_key: Option<String>,
        name: String,
    },
    /// Our favourite game servers (plan E7.2).
    ShowFavourites,
    /// The local activity feed (plan E7.15, TU1(a)): its first page.
    ShowActivity,
    /// A page of the activity feed: the first (`before` `None`) or the one
    /// older than row `before` (`ActivityList`).
    LoadActivity {
        before: Option<i64>,
    },
    /// The games we can launch (`InstalledGamesGet`, plan E7.15 IV3).
    ShowInstalledGames,
    /// The Servers view (`ServersView`, plan E7.2 G3).
    ShowServers,
    /// A subject's Infoview (`InfoviewGet` from `TUI_WINDOW`, plan E7.15
    /// IV7).
    ShowInfoview {
        subject: rekindle_types::dto::infoview::InfoviewSubject,
    },
    /// `RecentServerRemove`: forget a recently visited server (plan E7.2
    /// G3); the view comes back.
    RemoveRecentServer {
        game_id: u32,
        server: rekindle_types::game_server::ServerAddress,
    },
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
    /// `CreateForumPost`: a thread in a forum channel and its starter, one
    /// outcome (plan E3.3, FT1).
    CreateForumPost {
        community: String,
        channel: String,
        title: String,
        tag: Option<String>,
        body: String,
    },
    /// `ChannelMessageEdit`: our message's new body (plan E3.3 E1).
    EditChannelMessage {
        channel: String,
        message_id: String,
        text: String,
    },
    /// `ChannelMessageDelete`: our message, deleted for members (E3, E4).
    DeleteChannelMessage {
        channel: String,
        message_id: String,
    },
    /// `ChannelMessageAdminDelete`: another member's message, removed as a
    /// moderator (M1–M4).
    RemoveChannelMessage {
        community: String,
        channel: String,
        message_id: String,
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

    // ─── Group DMs (plan E2.6; the host reviews each) ──────────
    AcceptGroupInvite {
        group_id: String,
    },
    DeclineGroupInvite {
        group_id: String,
    },
    LeaveGroup {
        group_id: String,
    },
    SendGroupDm {
        group_id: String,
        text: String,
    },

    // ─── Favourite servers (plan E7.2) ──────────────────────────
    AddFavourite {
        game_id: u32,
        server: rekindle_types::game_server::ServerAddress,
        label: String,
    },
    RemoveFavourite {
        game_id: u32,
        server: rekindle_types::game_server::ServerAddress,
    },

    /// `GameLaunch` (lead decision (50)): `source` says where the server
    /// came from; the Host decides the rest and reviews a peer's server.
    LaunchGame {
        game_id: u32,
        server: Option<rekindle_types::game_server::ServerAddress>,
        source: rekindle_types::game_server::JoinSource,
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
    RefreshHeldReviews,
    HeldReviewsLoaded {
        generation: u64,
        reviews: Vec<rekindle_types::intent::HeldReview>,
    },
    /// The skin this device shows (`SkinGet`'s answer or the host's
    /// `SkinChanged`): the theme takes its palette (plan E7.7.2).
    ApplySkin(Box<rekindle_types::dto::skin_library::SkinApplied>),
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
    PermissionCatalogueLoaded(rekindle_types::dto::permission_catalogue::PermissionCatalogue),
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
    /// A DM thread's stored messages (`DmHistory`).
    DmHistoryLoaded {
        peer_key: String,
        messages: Vec<rekindle_types::dto::dm::DmMessage>,
    },
    /// Our groups and unanswered invites (`GroupDmList`, plan E2.6).
    GroupDmsLoaded {
        groups: Vec<rekindle_types::dto::group_dm::GroupDmView>,
    },
    /// A group's stored messages (`GroupDmMessages`).
    GroupMessagesLoaded {
        group_id: String,
        messages: Vec<rekindle_types::dto::dm::DmMessage>,
    },
    StatusLoaded {
        snapshot: rekindle_types::display::StatusSnapshot,
    },
    /// A community's members, grouped and ordered by the Host
    /// (`CommunityMembers`).
    CommunityMembersLoaded {
        community: String,
        members: Vec<rekindle_types::dto::community::CommunityMemberView>,
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
    /// The Host's network bars when the TUI started (`NetworkStatus`,
    /// plan E7.2), live after this from `NetworkEvent::Strength`.
    NetworkBarsLoaded {
        bars: u8,
    },
    /// A profile's bio and game time (plan E7.2): a friend's as they last
    /// shared them (`peer_key`), or ours (`None`).
    ProfileLoaded {
        peer_key: Option<String>,
        bio: Option<String>,
        totals: Vec<rekindle_types::dto::presence::GameTotalView>,
    },
    /// Our favourite game servers (`FavouriteServersGet`, plan E7.2).
    FavouritesLoaded {
        servers: Vec<rekindle_types::dto::presence::FavouriteServerView>,
    },
    /// A page of the activity feed (`ActivityList`, plan E7.15), newest
    /// first, asked with `before`.
    ActivityLoaded {
        before: Option<i64>,
        entries: Vec<rekindle_types::activity::ActivityListItem>,
    },
    /// The games we can launch (`InstalledGamesGet`, plan E7.15 IV3).
    InstalledGamesLoaded {
        games: Vec<rekindle_types::dto::presence::InstalledGameView>,
    },
    /// The Servers view as the Host composed it (`ServersView`, or the
    /// answer to `RecentServerRemove`, plan E7.2 G3).
    ServersViewLoaded {
        view: rekindle_types::dto::servers::ServersView,
    },
    /// A subject's Infoview as the Host composed it (`InfoviewGet`, plan
    /// E7.15 IV7).
    InfoviewLoaded {
        view: rekindle_types::dto::infoview::InfoviewView,
    },
}
