//! The community activities: scheduled events, their RSVPs and the game servers (plan E3.1), carried as `CommunityRequest::Activities`.

use serde::{Deserialize, Serialize};

use crate::message::SecurityLevel;
use crate::protocol::{AgentCapability, Lane};

/// A community activities request.
#[derive(Clone, Serialize, Deserialize)]
pub enum ActivityRequest {
    /// A community's events, by start time, with their attendees.
    EventList { community: String },

    /// One event's attendees.
    EventAttendees { community: String, event_id: String },

    /// Schedule an event; answered with its id.
    EventCreate {
        community: String,
        event: rekindle_types::event::NewEvent,
    },

    /// Change an event's fields.
    EventEdit {
        community: String,
        event_id: String,
        edit: rekindle_types::event::EventEdit,
    },

    /// Cancel an event (kept, marked cancelled).
    EventCancel { community: String, event_id: String },

    /// Delete an event.
    EventDelete { community: String, event_id: String },

    /// Our answer to an event: `going`, `interested` (or `maybe`), `declined`.
    EventRsvp {
        community: String,
        event_id: String,
        status: String,
    },

    /// A community's game servers, newest first.
    GameServerList { community: String },

    /// Add a game server to a community; answered with its id.
    GameServerAdd {
        community: String,
        game_id: String,
        label: String,
        address: String,
    },

    /// Remove a community's game server.
    GameServerRemove {
        community: String,
        server_id: String,
    },
}

impl ActivityRequest {
    /// The variant name, without its fields.
    #[must_use]
    pub fn name(&self) -> &'static str {
        self.class().0
    }

    pub(crate) fn class(&self) -> (&'static str, Lane, SecurityLevel) {
        use Lane::{Query, Write};
        use SecurityLevel::Authenticated;
        match self {
            Self::GameServerList { .. } => ("GameServerList", Query, Authenticated),
            Self::EventList { .. } => ("EventList", Query, Authenticated),
            Self::EventAttendees { .. } => ("EventAttendees", Query, Authenticated),
            Self::EventCreate { .. } => ("EventCreate", Write, Authenticated),
            Self::EventEdit { .. } => ("EventEdit", Write, Authenticated),
            Self::EventCancel { .. } => ("EventCancel", Write, Authenticated),
            Self::EventDelete { .. } => ("EventDelete", Write, Authenticated),
            Self::EventRsvp { .. } => ("EventRsvp", Write, Authenticated),
            Self::GameServerAdd { .. } => ("GameServerAdd", Write, Authenticated),
            Self::GameServerRemove { .. } => ("GameServerRemove", Write, Authenticated),
        }
    }

    /// The capability an `Agent`-level sender needs for this request, which every
    /// request of this domain has.
    #[must_use]
    pub fn agent_capability(&self) -> AgentCapability {
        use AgentCapability as C;
        match self {
            Self::GameServerList { .. } | Self::EventList { .. } | Self::EventAttendees { .. } => {
                C::CommunitiesRead
            }
            Self::GameServerAdd { .. }
            | Self::GameServerRemove { .. }
            | Self::EventCreate { .. }
            | Self::EventEdit { .. }
            | Self::EventCancel { .. }
            | Self::EventDelete { .. }
            | Self::EventRsvp { .. } => C::CommunitiesWrite,
        }
    }
}
