//! Transport notification types for the IPC boundary.
//!
//! These enums cross the daemon->CLI/TUI boundary. They carry enough
//! context for display decisions ("show a toast", "refresh the peer
//! list") without exposing transport internals.
//!
//! Defined here in `rekindle-types` so that both `rekindle-transport`
//! (producer) and `rekindle-cli` (consumer) can use them without the
//! CLI ever depending on the transport crate.

use serde::{Deserialize, Serialize};

// -- Attachment state --------------------------------------------------------

/// Network attachment state. Maps from Veilid's string representation.
///
/// Ordered by "goodness" -- higher discriminant values indicate stronger
/// attachment. This is a stable ABI contract consumed by CLI display code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum AttachmentState {
    Detached = 0,
    Attaching = 1,
    AttachedWeak = 2,
    AttachedGood = 3,
    AttachedStrong = 4,
    FullyAttached = 5,
    OverAttached = 6,
    Detaching = 7,
}

impl AttachmentState {
    /// Parse from Veilid's attachment state string representation.
    ///
    /// Unknown strings map to `Detached` (fail closed -- if we can't parse
    /// the state, we assume the worst).
    pub fn from_veilid_string(s: &str) -> Self {
        match s {
            "Detached" | "detached" => Self::Detached,
            "Attaching" | "attaching" => Self::Attaching,
            "AttachedWeak" | "attached_weak" => Self::AttachedWeak,
            "AttachedGood" | "attached_good" => Self::AttachedGood,
            "AttachedStrong" | "attached_strong" => Self::AttachedStrong,
            "FullyAttached" | "fully_attached" => Self::FullyAttached,
            "OverAttached" | "over_attached" => Self::OverAttached,
            "Detaching" | "detaching" => Self::Detaching,
            _ => Self::Detached,
        }
    }

    /// Whether this state represents an attached (usable) network.
    pub fn is_attached(self) -> bool {
        matches!(
            self,
            Self::AttachedWeak
                | Self::AttachedGood
                | Self::AttachedStrong
                | Self::FullyAttached
                | Self::OverAttached
        )
    }

    /// Human-readable label for display.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Detached => "Detached",
            Self::Attaching => "Attaching",
            Self::AttachedWeak => "AttachedWeak",
            Self::AttachedGood => "AttachedGood",
            Self::AttachedStrong => "AttachedStrong",
            Self::FullyAttached => "FullyAttached",
            Self::OverAttached => "OverAttached",
            Self::Detaching => "Detaching",
        }
    }

    /// Parse from a u8 discriminant. Returns `Detached` for out-of-range values.
    pub fn from_u8(raw: u8) -> Self {
        match raw {
            1 => Self::Attaching,
            2 => Self::AttachedWeak,
            3 => Self::AttachedGood,
            4 => Self::AttachedStrong,
            5 => Self::FullyAttached,
            6 => Self::OverAttached,
            7 => Self::Detaching,
            _ => Self::Detached,
        }
    }
}

impl std::fmt::Display for AttachmentState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

// -- Transport notification --------------------------------------------------

/// Broadcast notification from the transport layer to CLI/TUI consumers.
///
/// These are summaries of transport events -- not the full event payloads.
/// They carry enough context for display decisions without exposing
/// transport internals.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TransportNotification {
    /// Network attachment state changed.
    AttachmentChanged {
        state: AttachmentState,
        is_attached: bool,
        public_internet_ready: bool,
    },

    /// One or more local private routes died.
    LocalRoutesDied { count: usize },

    /// One or more remote peer routes died.
    RemoteRoutesDied { peer_keys: Vec<String> },

    /// A DHT watch expired or was cancelled.
    WatchDied { record_key: String },

    /// A DM was received from a verified peer.
    DmReceived {
        sender_key: String,
        sender_name: String,
        timestamp: u64,
    },

    /// A new channel message was posted (gossip MessageNotification).
    ///
    /// Carries metadata only -- the message body is in the DHT channel log.
    /// Consumers use this to trigger a history read for the new message.
    MessagePosted {
        community_id: String,
        channel_id: String,
        message_id: String,
        sender_pseudonym: String,
        lamport_ts: u64,
    },

    /// A member is typing in a channel (gossip TypingIndicator).
    ///
    /// Ephemeral -- not stored, expires after ~5 seconds if not refreshed.
    TypingStarted {
        community_id: String,
        channel_id: String,
        sender_pseudonym: String,
    },

    /// A member's presence changed (gossip PresenceUpdate).
    PresenceChanged {
        community_id: String,
        pseudonym_key: String,
        status: String,
        game_name: Option<String>,
    },

    /// A gossip control operation was received that doesn't have a
    /// dedicated notification variant.
    GossipControl {
        community_id: String,
        sender_pseudonym: String,
        lamport_ts: u64,
    },

    /// A DHT value changed (watch notification).
    ValueChanged {
        record_key: String,
        changed_subkeys: Vec<u32>,
    },

    /// MEK was rotated for a community/channel.
    MekRotated {
        community_id: String,
        channel_id: Option<String>,
        generation: u64,
    },

    /// A voice participant joined.
    VoiceJoin {
        community_id: String,
        channel_id: String,
        participant_key: String,
    },

    /// A voice participant left.
    VoiceLeave {
        community_id: String,
        channel_id: String,
        participant_key: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attachment_state_round_trip() {
        for raw in 0..=7u8 {
            let state = AttachmentState::from_u8(raw);
            assert_eq!(state as u8, raw);
        }
        assert_eq!(AttachmentState::from_u8(255), AttachmentState::Detached);
    }

    #[test]
    fn attachment_state_from_veilid_string() {
        assert_eq!(
            AttachmentState::from_veilid_string("FullyAttached"),
            AttachmentState::FullyAttached
        );
        assert_eq!(
            AttachmentState::from_veilid_string("garbage"),
            AttachmentState::Detached
        );
    }

    #[test]
    fn is_attached_correct() {
        assert!(!AttachmentState::Detached.is_attached());
        assert!(!AttachmentState::Attaching.is_attached());
        assert!(AttachmentState::AttachedWeak.is_attached());
        assert!(AttachmentState::AttachedGood.is_attached());
        assert!(AttachmentState::FullyAttached.is_attached());
        assert!(!AttachmentState::Detaching.is_attached());
    }

    #[test]
    fn notification_serializes_round_trip() {
        let notif = TransportNotification::MessagePosted {
            community_id: "abc".into(),
            channel_id: "general".into(),
            message_id: "msg1".into(),
            sender_pseudonym: "alice".into(),
            lamport_ts: 42,
        };
        let json = serde_json::to_string(&notif).unwrap();
        let parsed: TransportNotification = serde_json::from_str(&json).unwrap();
        match parsed {
            TransportNotification::MessagePosted { lamport_ts, .. } => {
                assert_eq!(lamport_ts, 42);
            }
            _ => panic!("wrong variant"),
        }
    }
}
