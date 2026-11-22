//! The Servers view (plan E7.2 G3, SK2-G design §3), carried as
//! `IpcRequest::Servers`: the view itself, and forgetting a server we
//! visited. Every leaf keeps its own name, as `TransferRequest`'s do: the
//! variants are short (`ServersRequest::View`) but each is renamed on the
//! wire, in `name()` and so in every registry (`COMMAND_TABLE`, the shell's
//! command map) to its leaf name, which is unique across `IpcRequest`.

use serde::{Deserialize, Serialize};

use rekindle_types::game_server::ServerAddress;

use crate::message::SecurityLevel;
use crate::protocol::{AgentCapability, Lane};

/// A Servers view request.
#[derive(Clone, Serialize, Deserialize)]
pub enum ServersRequest {
    /// Our favourites, the servers we recently visited, our friends'
    /// servers (presence and game invites) and each community's list,
    /// every row with the Host's Join, add-favourite and remove verdicts.
    /// Composed only from what we hold.
    #[serde(rename = "ServersView")]
    View,
    /// Forget a recently visited server. The view comes back.
    #[serde(rename = "RecentServerRemove")]
    RecentRemove { game_id: u32, server: ServerAddress },
}

impl ServersRequest {
    /// The variant name, without its fields.
    #[must_use]
    pub fn name(&self) -> &'static str {
        self.class().0
    }

    pub(crate) fn class(&self) -> (&'static str, Lane, SecurityLevel) {
        use Lane::{Query, Write};
        use SecurityLevel::Authenticated;
        match self {
            Self::View => ("ServersView", Query, Authenticated),
            Self::RecentRemove { .. } => ("RecentServerRemove", Write, Authenticated),
        }
    }

    /// The view reads what our own identity holds (as
    /// `FavouriteServersGet`); which servers we keep is the user's own, so
    /// no agent forgets one (as `FavouriteServerRemove`).
    #[must_use]
    pub fn agent_capability(&self) -> Option<AgentCapability> {
        match self {
            Self::View => Some(AgentCapability::IdentityRead),
            Self::RecentRemove { .. } => None,
        }
    }
}

impl std::fmt::Debug for ServersRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut d = f.debug_struct(self.name());
        // A server address is a peer's network location: only the game is
        // logged.
        if let Self::RecentRemove { game_id, .. } = self {
            d.field("game_id", game_id);
        }
        d.finish()
    }
}
