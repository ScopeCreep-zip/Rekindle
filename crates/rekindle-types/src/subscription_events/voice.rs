//! Voice channel events — join, leave, mute, deafen, mode, roster.

use serde::{Deserialize, Serialize};

/// Voice channel activity events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum VoiceEvent {
    /// A member joined a voice channel.
    /// Triggered by: gossip `ControlPayload::VoiceJoin`.
    Joined {
        community: String,
        channel: String,
        pseudonym: String,
    },
    /// A member left a voice channel.
    /// Triggered by: gossip `ControlPayload::VoiceLeave`.
    Left {
        community: String,
        channel: String,
        pseudonym: String,
    },
    /// The voice channel mode changed (e.g., stage mode, host change).
    /// Triggered by: gossip `ControlPayload::VoiceModeSwitch`.
    ModeChanged {
        community: String,
        channel: String,
        mode: String,
        host_pseudonym: Option<String>,
    },
    /// A participant's mute state changed.
    /// Triggered by: gossip `ControlPayload::VoiceMute`.
    MuteChanged {
        community: String,
        channel: String,
        target_pseudonym: String,
        muted: bool,
    },
    /// A participant's deafen state changed.
    /// Triggered by: gossip `ControlPayload::VoiceDeafen`.
    DeafenChanged {
        community: String,
        channel: String,
        target_pseudonym: String,
        deafened: bool,
    },
    /// Full voice roster update (authoritative participant list).
    /// Triggered by: gossip `ControlPayload::VoiceRoster`.
    RosterUpdated {
        community: String,
        channel: String,
        participant_count: usize,
    },
}
