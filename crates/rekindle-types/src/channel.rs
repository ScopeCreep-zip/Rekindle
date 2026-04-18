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

use crate::id::MessageId;

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
    },

    /// Add or remove a reaction on a message.
    Reaction {
        message_id: MessageId,
        /// Unicode emoji string or custom expression ID
        emoji: String,
        /// true = add, false = remove. CRDT: LWW per (voter, message_id, emoji).
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
    Delete {
        message_id: MessageId,
        lamport: u64,
    },

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

    /// Vote in a poll. CRDT: LWW per (voter, message_id).
    PollVote {
        message_id: MessageId,
        option_indices: Vec<u32>,
        lamport: u64,
    },

    /// Advertise that we have a file cached locally for peer download.
    AttachmentCached {
        /// BLAKE3 hash of the file content
        hash: String,
        lamport: u64,
    },

    /// Raise/lower hand in a stage channel.
    HandRaise {
        raised: bool,
        lamport: u64,
    },
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
            | Self::PollVote { lamport, .. }
            | Self::AttachmentCached { lamport, .. }
            | Self::HandRaise { lamport, .. } => *lamport,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_entry_serde_roundtrip() {
        let entry = ChannelEntry::Message {
            message_id: MessageId([7u8; 16]),
            content: vec![0xDE, 0xAD],
            mek_generation: 3,
            timestamp: 1710000000,
            lamport: 100,
            sequence: 5,
            reply_to: None,
            flags: 0,
        };
        let json = serde_json::to_string(&entry).unwrap();
        let back: ChannelEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(entry, back);
    }

    #[test]
    fn reaction_toggle() {
        let add = ChannelEntry::Reaction {
            message_id: MessageId([1u8; 16]),
            emoji: "👍".into(),
            added: true,
            lamport: 10,
        };
        let remove = ChannelEntry::Reaction {
            message_id: MessageId([1u8; 16]),
            emoji: "👍".into(),
            added: false,
            lamport: 11,
        };
        // LWW: remove (lamport 11) wins over add (lamport 10)
        assert!(remove.lamport() > add.lamport());
    }
}
