//! The audit log and the default notification level (plan E3.1), carried as `CommunityRequest::Audit`.

use serde::{Deserialize, Serialize};

use crate::message::SecurityLevel;
use crate::protocol::{AgentCapability, Lane};

/// A audit log and the default notification level request.
#[derive(Clone, Serialize, Deserialize)]
pub enum AuditRequest {
    /// A page of the community's audit log, newest first: the rows older
    /// than `before_timestamp` (a Lamport time) when given, at most `limit`.
    AuditLog {
        community: String,
        before_timestamp: Option<u64>,
        limit: u32,
    },

    /// The community's default notification level, if one is set.
    NotificationDefaultGet { community: String },

    /// Set the community's default notification level (`all`, `mentions`
    /// or `nothing`).
    NotificationDefaultSet { community: String, level: String },
}

impl AuditRequest {
    /// The variant name, without its fields.
    #[must_use]
    pub fn name(&self) -> &'static str {
        self.class().0
    }

    pub(crate) fn class(&self) -> (&'static str, Lane, SecurityLevel) {
        use Lane::{Query, Write};
        use SecurityLevel::Authenticated;
        match self {
            Self::AuditLog { .. } => ("AuditLog", Query, Authenticated),
            Self::NotificationDefaultGet { .. } => ("NotificationDefaultGet", Query, Authenticated),
            Self::NotificationDefaultSet { .. } => ("NotificationDefaultSet", Write, Authenticated),
        }
    }

    /// The capability an `Agent`-level sender needs for this request, which every
    /// request of this domain has.
    #[must_use]
    pub fn agent_capability(&self) -> AgentCapability {
        use AgentCapability as C;
        match self {
            Self::AuditLog { .. } | Self::NotificationDefaultGet { .. } => C::CommunitiesRead,
            Self::NotificationDefaultSet { .. } => C::CommunitiesWrite,
        }
    }
}
