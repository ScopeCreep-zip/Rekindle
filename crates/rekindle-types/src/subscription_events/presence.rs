//! Presence events — member status, game activity, away state.
//!
//! Presence is semi-ephemeral — entries expire after 5 minutes without
//! update. The subscription module auto-marks expired entries as offline.

use serde::{Deserialize, Serialize};

/// Presence change events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PresenceEvent {
    /// A community member's presence changed.
    /// Triggered by: gossip `PresenceUpdate`.
    CommunityMemberChanged {
        community: String,
        pseudonym: String,
        status: String,
        game_name: Option<String>,
        game_id: Option<u32>,
    },
    /// A DM peer's presence changed.
    /// Triggered by: `DmPayload::PresenceUpdate`.
    FriendChanged {
        peer_key: String,
        status: String,
        game_name: Option<String>,
    },
}
