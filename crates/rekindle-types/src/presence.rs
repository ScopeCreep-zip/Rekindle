//! v2.0 Member presence types for the SMPL member registry.
//!
//! Each member writes their own MemberPresence to their subkey in the
//! registry SMPL record. Presence is self-sovereign — no coordinator
//! approval needed to update your own status, display name, or profile.
//!
//! See architecture doc §4.3 Record 2 and §24.2 for profile fields.
//! See rekindle-architecture-v2.md §4.2 for field specifications.

use serde::{Deserialize, Serialize};

use crate::id::{ChannelId, EventId, PseudonymKey};

/// Member presence data written to the registry SMPL subkey.
///
/// Updated every 15 seconds by the heartbeat loop. Contains both
/// ephemeral state (status, voice channel, route blob) and profile
/// data (display name, bio, avatar).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MemberPresence {
    /// This member's community-specific pseudonym public key.
    pub pseudonym_key: PseudonymKey,

    /// Self-sovereign display name (no coordinator approval).
    pub display_name: Option<String>,

    /// "online", "away", "busy", "offline"
    pub status: String,

    /// Custom status text (e.g., "Playing Halo")
    pub custom_status: Option<String>,

    /// Currently joined voice channel (None if not in voice).
    pub current_voice_channel: Option<ChannelId>,

    /// Current private route blob for direct messaging.
    /// Refreshed every 120 seconds.
    pub route_blob: Vec<u8>,

    /// Unix timestamp of last heartbeat write.
    pub last_heartbeat: u64,

    /// Currently playing game (from rekindle-game-detect).
    pub game_info: Option<GameInfo>,

    /// Content-addressed avatar reference (BLAKE3 hash).
    pub avatar_ref: Option<String>,

    /// Short bio (max 190 chars).
    pub bio: Option<String>,

    /// Pronouns (max 40 chars).
    pub pronouns: Option<String>,

    /// Profile accent color (ARGB u32).
    pub theme_color: Option<u32>,

    /// Earned/assigned badge IDs.
    pub badges: Vec<String>,

    /// Whether currently in a call.
    pub in_call: bool,

    /// "audio", "video", "screen_share" (if in_call is true).
    pub call_type: Option<String>,

    /// Route blob for an opt-in push relay (Tier 3 notifications).
    pub push_relay_route: Option<Vec<u8>>,

    /// RSVPs for scheduled events.
    pub event_rsvps: Vec<EventRSVP>,

    /// Advertised message history ranges for mutual aid (shared lockers).
    pub history_ranges: Vec<HistoryRange>,
}

/// Game currently being played (populated by rekindle-game-detect).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GameInfo {
    pub game_name: String,
    pub game_id: Option<String>,
    pub elapsed_seconds: Option<u64>,
    pub server_address: Option<String>,
}

/// RSVP for a scheduled community event.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EventRSVP {
    pub event_id: EventId,
    /// "going", "interested", "declined"
    pub status: String,
}

/// Range of message history this member has cached locally.
/// Used by mutual aid: newcomers can request ranges from peers who have them.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HistoryRange {
    pub channel_id: ChannelId,
    pub oldest_lamport: u64,
    pub newest_lamport: u64,
}

impl Default for MemberPresence {
    fn default() -> Self {
        Self {
            pseudonym_key: PseudonymKey([0u8; 32]),
            display_name: None,
            status: "online".into(),
            custom_status: None,
            current_voice_channel: None,
            route_blob: Vec::new(),
            last_heartbeat: 0,
            game_info: None,
            avatar_ref: None,
            bio: None,
            pronouns: None,
            theme_color: None,
            badges: Vec::new(),
            in_call: false,
            call_type: None,
            push_relay_route: None,
            event_rsvps: Vec::new(),
            history_ranges: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presence_serde_roundtrip() {
        let presence = MemberPresence {
            pseudonym_key: PseudonymKey([0xAB; 32]),
            display_name: Some("FireStarter92".into()),
            status: "online".into(),
            route_blob: vec![1, 2, 3],
            last_heartbeat: 1710000000,
            ..Default::default()
        };
        let json = serde_json::to_string(&presence).unwrap();
        let back: MemberPresence = serde_json::from_str(&json).unwrap();
        assert_eq!(presence, back);
    }

    #[test]
    fn default_presence_is_online() {
        let p = MemberPresence::default();
        assert_eq!(p.status, "online");
        assert!(!p.in_call);
        assert!(p.route_blob.is_empty());
    }
}
