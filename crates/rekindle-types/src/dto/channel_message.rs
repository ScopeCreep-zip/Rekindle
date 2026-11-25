//! A community channel message as every frontend renders it (plan E3.1
//! E2c): the Host's `ChannelMessages` and `ChannelMessagesBefore` answer
//! with these, merged from our stored rows and the channel record.

use serde::{Deserialize, Serialize};

use crate::message::SendState;

/// The members who reacted to a message with one expression.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReactionGroup {
    pub emoji: String,
    pub count: u32,
    /// Their pseudonyms, ascending.
    pub reactors: Vec<String>,
}

/// One answer of a poll, with who chose it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessagePollAnswer {
    pub index: u8,
    pub text: String,
    pub vote_count: u32,
    /// The voters' pseudonyms, ascending.
    pub voters: Vec<String>,
}

/// A poll attached to a message, as the channel record's entries make it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessagePoll {
    pub poll_id: String,
    pub question: String,
    pub answers: Vec<MessagePollAnswer>,
    pub multi_select: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<u64>,
    pub closed: bool,
    /// Our own choices.
    #[serde(default)]
    pub selected_answers: Vec<u8>,
}

/// An attachment as a frontend sees it. The on-disk path of a downloaded
/// file stays in the backend (it carries the OS user name); a frontend
/// only learns that the file was saved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageAttachment {
    pub attachment_id: String,
    pub filename: String,
    pub mime_type: String,
    pub total_size: u64,
    pub chunk_count: u32,
    pub downloaded: bool,
}

/// A voice message as the receiving Host decoded it (plan E5.1, research
/// batch 7 V-2): the Ogg Opus audio, and the duration and waveform read
/// from that audio, never from the sender.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceMessageAudioView {
    /// The `audio/ogg` file, base64.
    #[serde(with = "crate::base64_bytes")]
    pub audio: Vec<u8>,
    pub duration_ms: u32,
    /// Peaks 0–255, base64.
    #[serde(with = "crate::base64_bytes")]
    pub waveform: Vec<u8>,
}

/// One channel message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelMessageView {
    /// Our stored row's id; 0 for a message read from the network only.
    pub id: i64,
    /// The sender's pseudonym.
    pub sender_id: String,
    pub body: String,
    /// The body did not open under the generation it names: a frontend
    /// must not render `body` as the sender's words.
    #[serde(default)]
    pub decryption_failed: bool,
    #[serde(default)]
    pub automod_blurred: bool,
    pub timestamp: i64,
    pub is_own: bool,
    /// The sender's message id.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_message_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reactions: Option<Vec<ReactionGroup>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pinned: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poll: Option<MessagePoll>,
    /// The original author's pseudonym when this message is a forward.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forwarded_from_author: Option<String>,
    /// That author as the community names them (the display form a
    /// sender's header shows); present only with `forwarded_from_author`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forwarded_from_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attachment: Option<MessageAttachment>,
    /// `ChannelEntry::Message.flags` (VOICE_MESSAGE=0x10,
    /// SUPPRESS_NOTIFICATIONS=0x20, …).
    #[serde(default)]
    pub flags: u32,
    /// Our own message's delivery state; `delivered` for a received one.
    pub send_state: SendState,
    /// Why our `failed` post was not delivered, as the user reads it (plan
    /// E3.2 S9, F9-3; S7l's key deadline), kept across a reload.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_reason: Option<String>,
    /// When its author last edited it, in seconds (plan E3.3), kept across
    /// a reload; absent for an unedited message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edited_at: Option<i64>,
    /// The moderator who removed it (plan E3.3 M4): its body is gone and
    /// every frontend shows "removed by"; absent otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub removed_by: Option<String>,
    /// A history-gap row, not a message (plan E3.3, D23): `sender_id`'s
    /// channel outbox reaches this chain seq and no read reached it, so
    /// some of its posts may be missing here; every frontend shows a gap
    /// line. Absent for a message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub history_gap_through: Option<u64>,
}
