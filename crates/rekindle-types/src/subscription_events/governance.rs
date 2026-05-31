//! Community governance events — metadata, channels, roles, invites, permissions.
//!
//! Triggered by gossip `ControlPayload` variants and DHT `ValueChange`
//! on the governance manifest record.

use serde::{Deserialize, Serialize};

/// Community governance change events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum GovernanceEvent {
    /// Community metadata changed (name, description, icon, join policy, etc.).
    /// Triggered by: gossip `GovernanceUpdated`, DHT watch on governance subkey 0.
    MetadataChanged { community: String },
    /// Channel list changed (added, removed, reordered, topic changed).
    /// Triggered by: DHT watch on governance subkey 1.
    ChannelsChanged { community: String },
    /// Role definitions changed (new role, permissions changed, deleted).
    /// Triggered by: DHT watch on governance subkey 3.
    RolesChanged { community: String },
    /// Ban list changed (member banned or unbanned).
    /// Triggered by: DHT watch on governance subkey 4.
    BansChanged { community: String },
    /// Invite list changed (invite created, used, expired).
    /// Triggered by: DHT watch on governance subkey 7.
    InvitesChanged { community: String },
    /// Channel permission overwrites changed.
    /// Triggered by: gossip `ControlPayload::ChannelOverwriteChanged`.
    ChannelPermissionsChanged { community: String, channel: String },
    /// Governance record updated (generic, from gossip `GovernanceUpdated`).
    /// Contains the subkey that changed for targeted re-reads.
    GovernanceSubkeyUpdated {
        community: String,
        subkey_index: u32,
        lamport_ts: u64,
    },
}
