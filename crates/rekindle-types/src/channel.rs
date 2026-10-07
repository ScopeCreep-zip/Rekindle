//! v2.0 Channel entry types for SMPL multi-writer channel records.
//!
//! Each member writes ChannelEntry variants to their own subkey in a
//! channel SMPL record (o_cnt: 0, same universal schema as governance).
//! Messages are merge-sorted by (lamport, author_pseudonym) across all
//! member subkeys to produce a deterministic total order.
//!
//! See architecture doc §4.3 Records 3+ and §16 for message features.
//! See rekindle-architecture-v2.md §4.3 for field specifications.

use serde::{Deserialize, Serialize};

use crate::attachment::AttachmentOffer;
use crate::id::MessageId;

/// Longest slowmode interval: Discord's `rate_limit_per_user` cap
/// (0-21600 seconds, six hours).
pub const MAX_SLOWMODE_SECONDS: u32 = 21_600;

/// Bitfield constants for `ChannelEntry::Message.flags`.
///
/// Per architecture §16.4: a message with the `VOICE_MESSAGE` flag carries
/// a single `audio/ogg` Lost Cargo attachment + waveform/duration metadata
/// in the (MEK-encrypted) body.
pub mod flags {
    /// Architecture §16.4 — voice message marker.
    pub const VOICE_MESSAGE: u32 = 0x10;
    /// Suppress OS push notification for this message.
    pub const SUPPRESS_NOTIFICATIONS: u32 = 0x20;
    /// Architecture §28.5 line 3111 — `mention_everyone` cleartext
    /// signal. Receivers route notifications without decrypting the
    /// body. Reader-validates: peers reject this bit from senders
    /// without `MENTION_EVERYONE` permission (§9.3).
    pub const MENTION_EVERYONE: u32 = 0x40;
    /// Architecture §28.5 — `@here` (online-only) cleartext signal.
    /// Same permission gate as `MENTION_EVERYONE`.
    pub const MENTION_HERE: u32 = 0x80;
}

/// All supported channel kinds.
///
/// Canonical definition — was duplicated verbatim as
/// `rekindle-protocol::dht::community::types::ChannelKind` (the wire
/// codec's copy) and `src-tauri::state::community::ChannelType` (the
/// Tauri frontend's copy, under its own locally-conventional name, plus
/// a now-pointless `From<ChannelKind> for ChannelType` bridging the
/// two). Both re-export this one instead; see `docs/research/
/// 2026-10-harvest-security-infra-audit.md` and
/// `xtask/src/main.rs::DUPLICATE_BODY_EXCEPTIONS` (plan 4.9).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChannelKind {
    Text,
    Voice,
    Announcement,
    Forum,
    Stage,
    Directory,
    Media,
    Events,
    Dm,
}

impl ChannelKind {
    /// Convert from the u8 wire representation.
    #[must_use]
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::Text),
            1 => Some(Self::Voice),
            2 => Some(Self::Announcement),
            3 => Some(Self::Forum),
            4 => Some(Self::Stage),
            5 => Some(Self::Directory),
            6 => Some(Self::Media),
            7 => Some(Self::Events),
            8 => Some(Self::Dm),
            _ => None,
        }
    }

    /// Convert to the u8 wire representation.
    #[must_use]
    pub fn to_u8(self) -> u8 {
        match self {
            Self::Text => 0,
            Self::Voice => 1,
            Self::Announcement => 2,
            Self::Forum => 3,
            Self::Stage => 4,
            Self::Directory => 5,
            Self::Media => 6,
            Self::Events => 7,
            Self::Dm => 8,
        }
    }

    /// String representation matching the serde lowercase format.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Voice => "voice",
            Self::Announcement => "announcement",
            Self::Forum => "forum",
            Self::Stage => "stage",
            Self::Directory => "directory",
            Self::Media => "media",
            Self::Events => "events",
            Self::Dm => "dm",
        }
    }
}

impl std::fmt::Display for ChannelKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for ChannelKind {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "text" => Ok(Self::Text),
            "voice" => Ok(Self::Voice),
            "announcement" => Ok(Self::Announcement),
            "forum" => Ok(Self::Forum),
            "stage" => Ok(Self::Stage),
            "directory" => Ok(Self::Directory),
            "media" => Ok(Self::Media),
            "events" => Ok(Self::Events),
            "dm" => Ok(Self::Dm),
            other => Err(format!("unknown channel kind: {other}")),
        }
    }
}

impl AsRef<str> for ChannelKind {
    /// Matches the Tauri frontend copy's `AsRef<str>` impl — kept so
    /// existing `.as_ref()` call sites don't need to change.
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

/// Entry written by a member to their subkey in a channel SMPL record.
///
/// All entries carry a `lamport` field for ordering. The `author` is
/// implicit — it's the pseudonym that owns the SMPL subkey.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ChannelEntry {
    /// A chat message. Content is MEK-encrypted ciphertext.
    Message {
        message_id: MessageId,
        /// AES-256-GCM ciphertext (encrypted with channel MEK)
        content: Vec<u8>,
        mek_generation: u64,
        timestamp: u64,
        lamport: u64,
        /// Per-author monotonic sequence for gap detection
        sequence: u64,
        reply_to: Option<MessageId>,
        /// Bitfield: VOICE_MESSAGE=0x10, SUPPRESS_NOTIFICATIONS=0x20, etc.
        flags: u32,
        /// Lost Cargo: optional file attachment offer (architecture §28.9
        /// line 3233 — "as part of a `ChannelEntry::Message`"). `None` for
        /// plain messages. Field is `#[serde(default, skip_serializing_if)]`
        /// so older serialized payloads (no `attachment` key) round-trip
        /// without a schema bump.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        attachment: Option<AttachmentOffer>,
    },

    /// Add or remove a reaction on a message.
    Reaction {
        message_id: MessageId,
        /// Unicode emoji string or "custom:{expression_id_hex}"
        expression: String,
        /// true = add, false = remove. CRDT: LWW per (voter, message_id, expression).
        added: bool,
        lamport: u64,
    },

    /// Edit a message. Only the original author's edits are honored.
    Edit {
        message_id: MessageId,
        /// New MEK-encrypted ciphertext
        new_ciphertext: Vec<u8>,
        lamport: u64,
    },

    /// Delete a message (tombstone). Irreversible in CRDT.
    Delete { message_id: MessageId, lamport: u64 },

    /// Forward a message from another channel/community.
    Forward {
        message_id: MessageId,
        original_message_id: MessageId,
        original_channel_id: [u8; 16],
        original_author: [u8; 32],
        /// Re-encrypted snapshot of original content
        content_snapshot: Vec<u8>,
        lamport: u64,
    },

    /// Create a poll attached to a message. CRDT: author-bound LWW per poll_id.
    PollCreate {
        poll_id: [u8; 16],
        message_id: MessageId,
        question: String,
        answers: Vec<String>,
        multi_select: bool,
        expires_at: Option<u64>,
        lamport: u64,
    },

    /// Vote in a poll. CRDT: LWW per (poll_id, voter).
    PollVote {
        poll_id: [u8; 16],
        selected_answers: Vec<u8>,
        lamport: u64,
    },

    /// Close a poll. CRDT: tombstone by poll_id.
    PollClose { poll_id: [u8; 16], lamport: u64 },

    /// Advertise that we have all-or-some chunks of a file cached locally
    /// (architecture §28.9 lines 3268-3274). The bitmap (plan §1.J4)
    /// resolves the spec's silence on partial-cache peers — downloaders
    /// route specific chunks to specific peers based on bitmap intersection.
    AttachmentCached {
        attachment_id: [u8; 16],
        /// LSB-first bit per chunk; length = `ceil(chunk_count / 8)`.
        chunk_bitmap: Vec<u8>,
        /// Total chunks in the file. Receivers reject entries whose bitmap
        /// length does not match `ceil(chunk_count / 8)`.
        chunk_count: u32,
        lamport: u64,
    },

    /// Raise/lower hand in a stage channel.
    HandRaise { raised: bool, lamport: u64 },
}

impl ChannelEntry {
    /// Extract the Lamport timestamp for ordering.
    pub fn lamport(&self) -> u64 {
        match self {
            Self::Message { lamport, .. }
            | Self::Reaction { lamport, .. }
            | Self::Edit { lamport, .. }
            | Self::Delete { lamport, .. }
            | Self::Forward { lamport, .. }
            | Self::PollCreate { lamport, .. }
            | Self::PollVote { lamport, .. }
            | Self::PollClose { lamport, .. }
            | Self::AttachmentCached { lamport, .. }
            | Self::HandRaise { lamport, .. } => *lamport,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_kind_roundtrip_u8() {
        for v in 0..=8u8 {
            let kind = ChannelKind::from_u8(v).unwrap();
            assert_eq!(kind.to_u8(), v);
        }
        assert!(ChannelKind::from_u8(9).is_none());
    }

    #[test]
    fn channel_kind_roundtrip_str() {
        let kinds = [
            "text",
            "voice",
            "announcement",
            "forum",
            "stage",
            "directory",
            "media",
            "events",
            "dm",
        ];
        for s in &kinds {
            let kind: ChannelKind = s.parse().unwrap();
            assert_eq!(kind.as_str(), *s);
            assert_eq!(kind.as_ref(), *s);
            assert_eq!(kind.to_string(), *s);
        }
    }

    #[test]
    fn channel_kind_serde_json() {
        let kind = ChannelKind::Forum;
        let json = serde_json::to_string(&kind).unwrap();
        assert_eq!(json, "\"forum\"");
        let back: ChannelKind = serde_json::from_str(&json).unwrap();
        assert_eq!(back, kind);
    }

    #[test]
    fn channel_entry_serde_roundtrip() {
        let entry = ChannelEntry::Message {
            message_id: MessageId([7u8; 16]),
            content: vec![0xDE, 0xAD],
            mek_generation: 3,
            timestamp: 1_710_000_000,
            lamport: 100,
            sequence: 5,
            reply_to: None,
            flags: 0,
            attachment: None,
        };
        let json = serde_json::to_string(&entry).unwrap();
        let back: ChannelEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(entry, back);
    }

    #[test]
    fn legacy_message_without_attachment_field_deserializes() {
        // Verifies the #[serde(default)] backward-compat: an old payload
        // produced before AttachmentOffer existed must still round-trip.
        let legacy_json = r#"{
            "type": "message",
            "message_id": [1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1],
            "content": [222, 173],
            "mek_generation": 3,
            "timestamp": 1710000000,
            "lamport": 100,
            "sequence": 5,
            "reply_to": null,
            "flags": 0
        }"#;
        let entry: ChannelEntry = serde_json::from_str(legacy_json).unwrap();
        match entry {
            ChannelEntry::Message { attachment, .. } => assert!(attachment.is_none()),
            _ => panic!("expected Message"),
        }
    }

    #[test]
    fn reaction_toggle() {
        let add = ChannelEntry::Reaction {
            message_id: MessageId([1u8; 16]),
            expression: "👍".into(),
            added: true,
            lamport: 10,
        };
        let remove = ChannelEntry::Reaction {
            message_id: MessageId([1u8; 16]),
            expression: "👍".into(),
            added: false,
            lamport: 11,
        };
        // LWW: remove (lamport 11) wins over add (lamport 10)
        assert!(remove.lamport() > add.lamport());
    }

    #[test]
    fn poll_entries_report_lamport() {
        let create = ChannelEntry::PollCreate {
            poll_id: [2u8; 16],
            message_id: MessageId([3u8; 16]),
            question: "Ready?".into(),
            answers: vec!["Yes".into(), "No".into()],
            multi_select: false,
            expires_at: None,
            lamport: 12,
        };
        let vote = ChannelEntry::PollVote {
            poll_id: [2u8; 16],
            selected_answers: vec![0],
            lamport: 13,
        };
        let close = ChannelEntry::PollClose {
            poll_id: [2u8; 16],
            lamport: 14,
        };

        assert_eq!(create.lamport(), 12);
        assert_eq!(vote.lamport(), 13);
        assert_eq!(close.lamport(), 14);
    }
}
