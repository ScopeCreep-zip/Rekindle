//! The Infoview (plan E7.15, IV4; Infoview design §2.2–§2.8): what the
//! Host answers `InfoviewGet` and pushes as `InfoviewEvent::Updated`. One
//! view for the desktop, the TUI and the CLI: the Host owns the subject,
//! the section list and its order, and the actions (memory: backend owns
//! policy). Every section holds only data already delivered to this
//! identity (design §3.1).
//!
//! A friend's view never lists communities (threat model I8, I11: no
//! mutual communities), and a community's rows never name which friends are
//! members (design §3.4).

use serde::{Deserialize, Serialize};

use crate::activity::ActivityListItem;
use crate::display::CommunityRemovalDisplay;
use crate::dto::community::PinnedMessageView;
use crate::dto::friends::Friend;
use crate::event::EventInfo;
use crate::friend::NotJoinableReason;

/// The window label the CLI asks `InfoviewGet` with (the desktop shell
/// sends each webview's own label).
pub const CLI_WINDOW: &str = "cli";

/// The window label the TUI asks `InfoviewGet` with: its one Infoview
/// view, distinct from the CLI's so a CLI run never takes over the pushes
/// the open TUI view folds (plan E7.15, IV7).
pub const TUI_WINDOW: &str = "tui";

/// What a window's Infoview shows: its selection, as the Host resolves it.
///
/// A pack's Info popup asks [`Selection`](Self::Selection) (plan E7.15,
/// IV5): the page follows its launcher's selection, as Xfire's did. A view
/// the Host answers or pushes always carries the resolved subject, never
/// `Selection`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "type"
)]
pub enum InfoviewSubject {
    /// Nothing selected: our own page (Xfire's "your activity report").
    Own,
    /// A friend (`user.tmpl`), by public key (hex).
    Friend { public_key: String },
    /// A friend group's header (`buddycat` / `customgroup`); `None` is the
    /// default group of friends in no group.
    FriendGroup { group_id: Option<i64> },
    /// A joined community (`clan.tmpl`), by anchor or name.
    Community { community: String },
    /// Whatever the asking window's launcher has selected (a popup's
    /// launcher; a window that is not a popup is its own): a friend is
    /// `Friend`, a friend group `FriendGroup`, anything else or nothing
    /// `Own`. The Host composes the page again when that selection changes.
    Selection,
}

/// One window's Infoview: its subject and its sections, in order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InfoviewView {
    pub subject: InfoviewSubject,
    pub sections: Vec<InfoviewSection>,
}

/// What a row's link does. Each one is a request (or a shell operation) the
/// frontend sends like any other action, so it passes the ADR 0019 intent
/// gate there: the Infoview never acts by itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "type"
)]
pub enum InfoviewAction {
    /// Open the community's window.
    OpenCommunity { community: String },
    /// Launch the game to the server (`host:port`). The Host states every
    /// row's `source`, so the window never decides where a server came from
    /// (memory: backend owns policy). A part the row lacks (no address
    /// shared, a community game id that is not numeric) is `None`.
    ///
    /// `joinable` and `joinable_reason` are the Host's verdict, the same
    /// pair `GameInfoState` carries: a friend's row copies the verdict their
    /// presence holds, a community's row is judged by
    /// `journal_entry::not_joinable` (no server, a game id that is not
    /// numeric, a game that cannot start on a server here). The window
    /// shows the row's Join always, disabled with the reason when it is
    /// not joinable.
    JoinServer {
        game_id: Option<u32>,
        server: Option<String>,
        source: JoinSource,
        joinable: bool,
        joinable_reason: Option<NotJoinableReason>,
    },
    /// Open the 1:1 chat with the friend.
    OpenChat { public_key: String },
    /// Open the friend's profile.
    OpenProfile { public_key: String },
    /// The feed's next page (`ActivityList` after row `before`).
    ActivityMore { before: i64 },
}

/// Where a `JoinServer` row's server came from, stated by the Host.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum JoinSource {
    /// A friend's presence shared it (a `NowPlaying` row).
    FriendPresence,
    /// A community lists it (a `GameServers` row).
    Community,
}

/// A joined community on our own page (design §2.4): the per-channel unread
/// counts summed by the Host.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InfoviewCommunityRow {
    pub anchor: String,
    pub name: String,
    pub unread: u32,
    pub mentions: u32,
    /// The next event not yet ended, when one is planned.
    pub next_event: Option<EventInfo>,
    pub action: InfoviewAction,
}

/// A friend playing now (design §2.5): the friend's buddy-list view, with
/// and `Join`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InfoviewPlayingRow {
    pub friend: Friend,
    pub game_id: u32,
    pub game_name: String,
    pub icon_key: Option<String>,
    pub elapsed_seconds: u32,
    /// The server label the friend shared, when they did.
    pub server_info: Option<String>,
    /// `JoinServer` with `JoinSource::FriendPresence`, always present.
    pub join: InfoviewAction,
}

/// A community's game server (design §2.8, Xfire's "Favorite Servers").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InfoviewServerRow {
    pub label: String,
    pub address: String,
    pub game_id: String,
    /// `JoinServer` with `JoinSource::Community`, always present.
    pub join: InfoviewAction,
}

/// One section of a view.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "type"
)]
pub enum InfoviewSection {
    /// The local activity feed, newest first (design §2.3); `more` is
    /// `ActivityMore` when older entries may follow.
    Activity {
        entries: Vec<ActivityListItem>,
        more: Option<InfoviewAction>,
    },
    /// Our communities with their unread counts (design §2.4).
    Communities { rows: Vec<InfoviewCommunityRow> },
    /// Friends in game, one row each (design §2.5, §2.7).
    NowPlaying { rows: Vec<InfoviewPlayingRow> },
    /// A friend's header (design §2.7): their buddy-list view, whether we
    /// block them, and the chat and profile links.
    FriendHeader {
        friend: Box<Friend>,
        blocked: bool,
        actions: Vec<InfoviewAction>,
    },
    /// A friend's bio, as they sent it.
    Bio { text: String },
    /// A friend group's members (design §2.8).
    GroupMembers {
        label: String,
        online: Vec<Friend>,
        offline: Vec<Friend>,
    },
    /// A community's header.
    CommunityHeader {
        anchor: String,
        name: String,
        description: String,
        member_count: u32,
        /// The members whose beats show them present now.
        online_count: u32,
        removed: Option<CommunityRemovalDisplay>,
    },
    /// A community's pinned messages, newest pin first (Xfire's clan
    /// "News", design §2.8), across the channels we may view.
    Pinned { pins: Vec<PinnedMessageView> },
    /// A community's events not yet ended, soonest first.
    Events { events: Vec<EventInfo> },
    /// A community's game servers.
    GameServers { servers: Vec<InfoviewServerRow> },
}

/// The Infoview's push (ADR 0013: to the one window it is for).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum InfoviewEvent {
    /// `window`'s view, composed again after the events it shows changed,
    /// at most once per window per coalescing period.
    Updated { window: String, view: InfoviewView },
}
