//! Moderation sub-states of `GovernanceState`: a moderator's message
//! delete, a member timeout, the community's join policy and its auto-mod
//! rules (architecture §10.7, §20.6; plan E3.3 M4).

use crate::op::OpStamp;
use rekindle_types::id::{ChannelId, PseudonymKey};

/// A moderator's delete of a channel message (`AdminDelete`, plan E3.3
/// M4): the moderator a reader names ("removed by"), the channel the
/// delete was judged in and the post's author it names (M1: readers apply
/// it only to a post by that author).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdminDeleteState {
    pub by: PseudonymKey,
    pub channel_id: ChannelId,
    pub author: PseudonymKey,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TimeoutState {
    pub duration_seconds: u64,
    pub started_at: u64,
    /// The reason the `TimeoutEntry` gave, bounded with the entry
    /// (`validate/bounds.rs`), shown with `MembershipEvent::TimedOut`.
    pub reason: Option<String>,
    pub stamp: OpStamp,
}

/// Architecture §10.7 + §20.6 — merged-state form of `CommunityPolicy`.
#[derive(Debug, Clone, PartialEq)]
pub struct CommunityPolicyState {
    pub policy_text: Option<String>,
    pub max_joins_per_interval: u32,
    pub join_interval_seconds: u32,
    pub stamp: OpStamp,
}

impl CommunityPolicyState {
    /// Defaults from architecture §20.6 — used when no `CommunityPolicy`
    /// entry has been merged yet.
    pub const DEFAULT_MAX_JOINS_PER_INTERVAL: u32 = 20;
    pub const DEFAULT_JOIN_INTERVAL_SECONDS: u32 = 600;
}

impl TimeoutState {
    /// Check if this timeout has expired given a current unix timestamp.
    pub fn is_expired(&self, now_unix: u64) -> bool {
        now_unix >= self.started_at.saturating_add(self.duration_seconds)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AutoModRuleState {
    pub name: String,
    pub enabled: bool,
    pub trigger_json: String,
    pub action: rekindle_types::governance::AutoModAction,
    pub stamp: OpStamp,
}
