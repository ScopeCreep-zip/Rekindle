//! The Servers view (plan E7.2 G3, SK2-G design §3; Xfire's Servers tab):
//! what the Host answers `ServersView` and pushes as
//! `SystemEvent::ServersViewChanged`. One view for the desktop, the TUI and
//! the CLI. Every row is built from what this identity already holds, our
//! favourites, the servers we visited, our accepted friends' presence and
//! game invites, our communities' lists: there is no central directory
//! (threat model "no central server").
//!
//! Each row carries its actions with the Host's verdict, always present: a
//! window shows Join, Add to favourites and Remove on every row, disabled
//! with the reason when the verdict refuses (memory: backend owns policy).

use serde::{Deserialize, Serialize};

use crate::friend::NotJoinableReason;
use crate::game_server::JoinSource;

/// The whole view: favourites, recent, friends, then one category per
/// community we are in, in that order. An empty category is still sent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServersView {
    pub categories: Vec<ServerCategory>,
    /// Whether `ServersRefresh` may start now: refused (`Refreshing`) while
    /// one runs (plan E7.2 G4).
    pub refresh: ServerVerdict,
}

/// What `ServersRefresh` answers: `started` is false when a refresh was
/// already running (plan E7.2 G4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RefreshAnswer {
    pub started: bool,
}

/// A server as the last refresh found it (plan E7.2 G4). A server is
/// queried only on the user's Refresh: a query shows our IP address to it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum ServerInfo {
    /// No refresh asked about it this session.
    NotQueried,
    /// A refresh is asking now.
    Querying,
    /// Its answer.
    #[serde(rename_all = "camelCase")]
    Answered {
        name: String,
        map: String,
        players: u32,
        max_players: u32,
        ping_ms: u32,
    },
    /// Why there was no answer, in the Host's words ("No reply within
    /// 2 s", "This game has no server query").
    #[serde(rename_all = "camelCase")]
    Unanswered { reason: String },
}

/// Which list a category is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ServerCategoryKind {
    /// Our `favourite_servers` (rt#259).
    Favourites,
    /// Servers we played on or launched to (`recent_servers`, rt#406).
    Recent,
    /// Our accepted friends' servers: their presence and their game invites
    /// (rt#260).
    Friends,
    /// One community's game-server list.
    Community,
}

/// A community a `Community` category lists.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerCommunity {
    pub anchor: String,
    pub name: String,
}

/// One category and its rows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerCategory {
    pub kind: ServerCategoryKind,
    /// Set for `Community` only.
    pub community: Option<ServerCommunity>,
    pub rows: Vec<ServerRow>,
}

/// The friend a `Friends` row came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerFriend {
    pub public_key: String,
    pub display_name: String,
}

/// Why a row's action is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ServerRefusal {
    /// The server is already one of our favourites.
    AlreadyFavourite,
    /// A community entry whose game id is not a number Rekindle knows.
    NoNumericGame,
    /// The address is not a `ServerAddress` (host:port).
    BadAddress,
    /// Only a favourite or a recent row can be removed.
    NotRemovable,
    /// A refresh is running (`ServersView.refresh`).
    Refreshing,
}

/// A row action's verdict, on the wire `{allowed: true}` or `{allowed:
/// false, reason}`: a refusal always names its reason, an allowed action
/// never carries one (the `{flag, reason}` pair `NotJoinableReason` uses,
/// made exact by type).
pub type ServerVerdict = super::verdict::ActionVerdict<ServerRefusal>;

/// Whether a row's Join can start: on the wire `{joinable: true}` or
/// `{joinable: false, joinableReason}` (flattened into [`ServerJoin`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RawJoinability", into = "RawJoinability")]
pub enum Joinability {
    Joinable,
    NotJoinable { reason: NotJoinableReason },
}

impl Joinability {
    /// From the Host's launch check (`None` = joinable).
    #[must_use]
    pub const fn of(reason: Option<NotJoinableReason>) -> Self {
        match reason {
            None => Self::Joinable,
            Some(reason) => Self::NotJoinable { reason },
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawJoinability {
    joinable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    joinable_reason: Option<NotJoinableReason>,
}

impl TryFrom<RawJoinability> for Joinability {
    type Error = &'static str;

    fn try_from(raw: RawJoinability) -> Result<Self, Self::Error> {
        match (raw.joinable, raw.joinable_reason) {
            (true, None) => Ok(Self::Joinable),
            (false, Some(reason)) => Ok(Self::NotJoinable { reason }),
            (true, Some(_)) => Err("a joinable server carries no reason"),
            (false, None) => Err("a server not joinable names its reason"),
        }
    }
}

impl From<Joinability> for RawJoinability {
    fn from(j: Joinability) -> Self {
        match j {
            Joinability::Joinable => Self {
                joinable: true,
                joinable_reason: None,
            },
            Joinability::NotJoinable { reason } => Self {
                joinable: false,
                joinable_reason: Some(reason),
            },
        }
    }
}

/// A row's Join: sent as `GameLaunch{game_id, server, source:
/// launch_source}` when joinable. The verdict is the Host's launch check
/// (`journal_entry::not_joinable`); a friend's row copies the verdict their
/// presence holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerJoin {
    pub game_id: Option<u32>,
    pub server: Option<String>,
    pub launch_source: JoinSource,
    #[serde(flatten)]
    pub joinability: Joinability,
}

/// One server in a category.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerRow {
    /// `None`: a community entry whose game id is not numeric.
    pub game_id: Option<u32>,
    pub game_name: String,
    /// `src/assets/game-icons/<key>.png`, from the game database.
    pub icon_key: Option<String>,
    /// host:port, as stored or shared.
    pub address: String,
    /// A favourite's or community entry's label; empty otherwise.
    pub label: String,
    /// Where the row came from (`Recent` for a visited server).
    pub source: JoinSource,
    /// Set for a `Friends` row.
    pub friend: Option<ServerFriend>,
    /// A recent row's last visit, unix seconds.
    pub last_seen: Option<i64>,
    pub join: ServerJoin,
    /// What the last refresh learned of the server (plan E7.2 G4).
    pub info: ServerInfo,
    /// `FavouriteServerAdd` from this row.
    pub add_favourite: ServerVerdict,
    /// `FavouriteServerRemove` (a favourite) or `RecentServerRemove` (a
    /// recent row).
    pub remove: ServerVerdict,
}

#[cfg(test)]
#[path = "servers_tests.rs"]
mod tests;
