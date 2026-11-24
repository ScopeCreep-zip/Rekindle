//! A group DM's game lobby (plan E7.2 G6, SK2-G design §5), carried as
//! `IpcRequest::Lobby`. One member hosts a lobby for the game it shows, the
//! other members join or leave it, and the host launches it on a server or
//! closes it; every member folds the same `Lobby*` frames, so all of them
//! show the same lobby. Every leaf keeps its own name, as `ServersRequest`'s
//! do: the variants are short (`LobbyRequest::Host`) but each is renamed on
//! the wire, in `name()` and so in every registry (`COMMAND_TABLE`, the
//! shell's command map) to its leaf name, which is unique across
//! `IpcRequest`.

use serde::{Deserialize, Serialize};

use rekindle_types::game_server::ServerAddress;

use crate::message::SecurityLevel;
use crate::protocol::{AgentCapability, Lane};

/// A group DM's game-lobby request.
#[derive(Clone, Serialize, Deserialize)]
pub enum LobbyRequest {
    /// Open a game lobby in `group_id` for the game we show now (plan E7.2
    /// G6): refused when we show no game, when the group already shows an
    /// open lobby, or when it is not a group we hold. Every other member is told
    /// through the group's chain and shows it with Join enabled. Reversed by
    /// `LobbyClose` (`ReversibleByInverse`).
    #[serde(rename = "LobbyHost")]
    Host { group_id: String },
    /// Join `lobby_id` (32 hex), another member's open lobby in
    /// `group_id`: refused for our own lobby, a lobby already launched or
    /// closed, or one we are already in. Reversed by `LobbyLeave`.
    #[serde(rename = "LobbyJoin")]
    Join { group_id: String, lobby_id: String },
    /// Leave `lobby_id`, a lobby we joined and do not host; the host closes
    /// its own lobby instead (`LobbyClose`).
    #[serde(rename = "LobbyLeave")]
    Leave { group_id: String, lobby_id: String },
    /// Close `lobby_id`, a lobby we host and have not closed: every member
    /// shows it closed, and no one can join it again.
    #[serde(rename = "LobbyClose")]
    Close { group_id: String, lobby_id: String },
    /// Launch `lobby_id`, a lobby we host with at least one member, on
    /// `server` (`None`: the server our shown game reports, refused when it
    /// reports none). TrustChanging (`LaunchGameLobby`): every member is
    /// sent the address, so the review names it before anything leaves us.
    #[serde(rename = "LobbyLaunch")]
    Launch {
        group_id: String,
        lobby_id: String,
        server: Option<ServerAddress>,
    },
    /// The lobby `group_id` shows now (`null` when none), with the Host's
    /// Join, Leave, Launch and Close verdicts for us; composed only from
    /// what we hold.
    #[serde(rename = "LobbyGet")]
    Get { group_id: String },
}

impl LobbyRequest {
    /// The variant name, without its fields.
    #[must_use]
    pub fn name(&self) -> &'static str {
        self.class().0
    }

    pub(crate) fn class(&self) -> (&'static str, Lane, SecurityLevel) {
        use Lane::{Query, Write};
        use SecurityLevel::Authenticated;
        match self {
            Self::Host { .. } => ("LobbyHost", Write, Authenticated),
            Self::Join { .. } => ("LobbyJoin", Write, Authenticated),
            Self::Leave { .. } => ("LobbyLeave", Write, Authenticated),
            Self::Close { .. } => ("LobbyClose", Write, Authenticated),
            Self::Launch { .. } => ("LobbyLaunch", Write, Authenticated),
            Self::Get { .. } => ("LobbyGet", Query, Authenticated),
        }
    }

    /// The lobby is read as the group's history is, and written as its
    /// messages are.
    #[must_use]
    pub fn agent_capability(&self) -> AgentCapability {
        match self {
            Self::Get { .. } => AgentCapability::ImHistory,
            Self::Host { .. }
            | Self::Join { .. }
            | Self::Leave { .. }
            | Self::Close { .. }
            | Self::Launch { .. } => AgentCapability::ImWrite,
        }
    }
}

impl std::fmt::Debug for LobbyRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut d = f.debug_struct(self.name());
        match self {
            Self::Host { group_id } | Self::Get { group_id } => {
                d.field("group_id", group_id);
            }
            Self::Join { group_id, lobby_id }
            | Self::Leave { group_id, lobby_id }
            | Self::Close { group_id, lobby_id } => {
                d.field("group_id", group_id).field("lobby_id", lobby_id);
            }
            // A server address is a peer's network location: only whether
            // one was named is logged.
            Self::Launch {
                group_id,
                lobby_id,
                server,
            } => {
                d.field("group_id", group_id)
                    .field("lobby_id", lobby_id)
                    .field("server_named", &server.is_some());
            }
        }
        d.finish()
    }
}
