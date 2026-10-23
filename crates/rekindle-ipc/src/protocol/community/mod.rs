//! The community-domain requests (plan E3.1), carried as
//! `IpcRequest::Community`: one sub-enum per domain, each in its own file.
//! Every leaf variant keeps its own name: `IpcRequest::name` answers the
//! leaf, so the intent table, the lanes and the audit log name it as before.

mod activities;
mod audit;
mod channels;
mod expressions;
mod lifecycle;
mod members;
mod messages;
mod pins;
mod rich;
mod settings;
mod unread;

pub use activities::ActivityRequest;
pub use audit::AuditRequest;
pub use channels::ChannelRequest;
pub use expressions::ExpressionRequest;
pub use lifecycle::LifecycleRequest;
pub use members::MemberRequest;
pub use messages::ChannelMessageRequest;
pub use pins::PinRequest;
pub use rich::RichRequest;
pub use settings::SettingsRequest;
pub use unread::UnreadRequest;

use serde::{Deserialize, Serialize};

use crate::message::SecurityLevel;
use crate::protocol::{AgentCapability, Lane};

/// A community-domain request: one variant per domain. Every leaf is
/// matched in its domain's handler in the daemon without a wildcard arm.
#[derive(Clone, Serialize, Deserialize)]
pub enum CommunityRequest {
    /// Communities: create, join, leave, the lists, the waiting room, ownership and invites.
    Lifecycle(LifecycleRequest),
    /// Community settings: our per-community profile and pictures, the policy, onboarding and the welcome screen.
    Settings(SettingsRequest),
    /// Channels and categories: their list, creation, edits, order and overwrites.
    Channels(ChannelRequest),
    /// Channel messages: sending, editing, deleting, forwarding, history, typing, the active channel and the channel keys.
    Messages(ChannelMessageRequest),
    /// Read markers and unread counts.
    Unread(UnreadRequest),
    /// Audit log and the default notification level.
    Audit(AuditRequest),
    /// Community activities: scheduled events, their RSVPs and the game servers.
    Activities(ActivityRequest),
    /// Member management: roles and their assignment, kicks, bans, timeouts, member blocks, automod and mod reports.
    Members(MemberRequest),
    /// Pins and search.
    Pins(PinRequest),
    /// Reactions, threads and polls.
    Rich(RichRequest),
    /// Custom emoji and stickers.
    Expressions(ExpressionRequest),
}

impl CommunityRequest {
    /// The leaf variant name, without its fields.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Lifecycle(request) => request.name(),
            Self::Settings(request) => request.name(),
            Self::Channels(request) => request.name(),
            Self::Messages(request) => request.name(),
            Self::Unread(request) => request.name(),
            Self::Audit(request) => request.name(),
            Self::Activities(request) => request.name(),
            Self::Members(request) => request.name(),
            Self::Pins(request) => request.name(),
            Self::Rich(request) => request.name(),
            Self::Expressions(request) => request.name(),
        }
    }

    pub(crate) fn class(&self) -> (&'static str, Lane, SecurityLevel) {
        match self {
            Self::Lifecycle(request) => request.class(),
            Self::Settings(request) => request.class(),
            Self::Channels(request) => request.class(),
            Self::Messages(request) => request.class(),
            Self::Unread(request) => request.class(),
            Self::Audit(request) => request.class(),
            Self::Activities(request) => request.class(),
            Self::Members(request) => request.class(),
            Self::Pins(request) => request.class(),
            Self::Rich(request) => request.class(),
            Self::Expressions(request) => request.class(),
        }
    }

    /// The capability an `Agent`-level sender needs for this request, or
    /// `None` when no agent may send it.
    #[must_use]
    pub fn agent_capability(&self) -> Option<AgentCapability> {
        match self {
            Self::Lifecycle(request) => request.agent_capability(),
            Self::Settings(request) => Some(request.agent_capability()),
            Self::Channels(request) => Some(request.agent_capability()),
            Self::Messages(request) => request.agent_capability(),
            Self::Unread(request) => Some(request.agent_capability()),
            Self::Audit(request) => Some(request.agent_capability()),
            Self::Activities(request) => Some(request.agent_capability()),
            Self::Members(request) => Some(request.agent_capability()),
            Self::Pins(request) => request.agent_capability(),
            Self::Rich(request) => Some(request.agent_capability()),
            Self::Expressions(request) => Some(request.agent_capability()),
        }
    }
}
