//! What a `GovernanceEntry::ModReport` names (plan E3.1, D26): a channel
//! message or a member, reported to the community's moderators.

use serde::{Deserialize, Serialize};

use crate::id::{ChannelId, PseudonymKey};

/// The subject of a moderator report.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ModReportTarget {
    /// One post in one channel.
    Message {
        channel_id: ChannelId,
        message_id: String,
    },
    /// A member, by community pseudonym.
    Member { pseudonym: PseudonymKey },
}
