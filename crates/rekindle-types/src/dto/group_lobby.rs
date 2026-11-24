//! A group DM's game lobby as the Host shows it (plan E7.2 G6, SK2-G §5):
//! the fold of the members' lobby entries (`group_lobbies`), with each
//! action's verdict decided by the Host from its own state, so no window
//! decides what it may do.

use serde::{Deserialize, Serialize};

use crate::game_server::ServerAddress;

/// One lobby of a group.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LobbyView {
    /// The host's 16 random bytes, in hex.
    pub lobby_id: String,
    pub host_key: String,
    pub game_id: u32,
    /// The joined members (never the host), in key order.
    pub members: Vec<String>,
    pub state: LobbyStatus,
    /// `LobbyJoin`: an open lobby another member hosts that we are not in.
    pub join: LobbyVerdict,
    /// `LobbyLeave`: we joined it and are not its host.
    pub leave: LobbyVerdict,
    /// `LobbyLaunch`: we host it, it is open and holds at least one member.
    pub launch: LobbyVerdict,
    /// `LobbyClose`: we host it and it is not closed.
    pub close: LobbyVerdict,
}

/// Where a lobby is in its life.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
pub enum LobbyStatus {
    /// Members may join and leave.
    Open,
    /// The host launched it on `server`; a member in it launches its own
    /// game there only on its own confirmation (`GameLaunch{source: Lobby}`).
    Launched { server: ServerAddress },
    /// The host closed it.
    Closed,
}

/// Why a lobby action is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LobbyRefusal {
    /// The lobby is launched or closed.
    NotOpen,
    /// The lobby is already closed.
    Closed,
    /// We host it: the host neither joins nor leaves, it closes.
    OwnLobby,
    /// Only its host launches or closes it.
    NotHost,
    /// We are already in it.
    AlreadyIn,
    /// We are not in it.
    NotIn,
    /// No member joined yet.
    NoMembers,
}

/// A lobby action's verdict, on the wire `{allowed: true}` or `{allowed:
/// false, reason}` (`ServerVerdict`'s shape).
pub type LobbyVerdict = super::verdict::ActionVerdict<LobbyRefusal>;
