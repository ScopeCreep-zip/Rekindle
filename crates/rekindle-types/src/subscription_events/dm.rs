//! DM lifecycle subscription events.

use serde::{Deserialize, Serialize};

/// DM conversation lifecycle events (invite, decline, leave).
/// Distinct from FriendEvent — friends are already established when
/// DM invites occur.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DmLifecycleEvent {
    /// An inbound DM invite arrived. UI should prompt accept/decline.
    InviteReceived {
        record_key: String,
        sender_pseudonym: String,
        sender_public_key_hex: String,
        is_group: bool,
    },
    /// A peer declined our DM invite.
    InviteDeclined {
        record_key: String,
        reason: String,
    },
    /// A peer left a group DM.
    MemberLeft {
        record_key: String,
        sender_public_key_hex: String,
    },
}
