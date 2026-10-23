//! The pins and search (plan E3.1), carried as `CommunityRequest::Pins`.

use serde::{Deserialize, Serialize};

use crate::message::SecurityLevel;
use crate::protocol::{AgentCapability, Lane};

/// A pins and search request.
#[derive(Clone, Serialize, Deserialize)]
pub enum PinRequest {
    /// A channel's pinned messages.
    ChannelPins {
        community: String,
        channel_id: String,
    },

    /// Pin a channel message (`MANAGE_MESSAGES`).
    MessagePin {
        community: String,
        channel_id: String,
        message_id: String,
    },

    /// Unpin a channel message (`MANAGE_MESSAGES`).
    MessageUnpin {
        community: String,
        channel_id: String,
        message_id: String,
    },

    /// Pin or unpin an attachment, exempting it from cache eviction
    /// (`MANAGE_COMMUNITY`).
    AttachmentPin {
        community: String,
        attachment_id: String,
        pinned: bool,
    },

    /// Upload the file at `path` (absolute, the shell's pick) to a channel:
    /// the new attachment id. Never an agent's: the Host would read a local
    /// file (plan E5.1).
    AttachmentUpload {
        community: String,
        channel_id: String,
        path: String,
    },

    /// Download an attachment to `path` (absolute, the shell's pick). Never
    /// an agent's: the Host would write a local file (plan E5.1).
    AttachmentDownload {
        community: String,
        channel_id: String,
        attachment_id: String,
        path: String,
    },

    /// The stored record of an attachment (its name, size, where it was
    /// saved).
    AttachmentInfo {
        community: String,
        channel_id: String,
        attachment_id: String,
    },

    /// Send a recorded voice message: the new attachment id.
    VoiceMessageSend {
        community: String,
        channel_id: String,
        opus: Vec<u8>,
        duration_ms: u32,
        waveform: Vec<u8>,
    },

    /// A voice message's audio bytes, fetched into the chunk cache.
    VoiceMessageAudio {
        community: String,
        channel_id: String,
        attachment_id: String,
    },

    /// Full-text search over the stored messages.
    MessageSearch {
        request: rekindle_types::search::MessageSearch,
    },
}

impl PinRequest {
    /// The variant name, without its fields.
    #[must_use]
    pub fn name(&self) -> &'static str {
        self.class().0
    }

    pub(crate) fn class(&self) -> (&'static str, Lane, SecurityLevel) {
        use Lane::{Query, Write};
        use SecurityLevel::Authenticated;
        match self {
            Self::ChannelPins { .. } => ("ChannelPins", Query, Authenticated),
            Self::MessagePin { .. } => ("MessagePin", Write, Authenticated),
            Self::MessageUnpin { .. } => ("MessageUnpin", Write, Authenticated),
            Self::AttachmentPin { .. } => ("AttachmentPin", Write, Authenticated),
            Self::AttachmentUpload { .. } => ("AttachmentUpload", Write, Authenticated),
            Self::AttachmentDownload { .. } => ("AttachmentDownload", Write, Authenticated),
            Self::AttachmentInfo { .. } => ("AttachmentInfo", Query, Authenticated),
            Self::VoiceMessageSend { .. } => ("VoiceMessageSend", Write, Authenticated),
            Self::VoiceMessageAudio { .. } => ("VoiceMessageAudio", Query, Authenticated),
            Self::MessageSearch { .. } => ("MessageSearch", Query, Authenticated),
        }
    }

    /// The capability an `Agent`-level sender needs for this request, or
    /// `None` when no agent may send it: an upload or a download names a
    /// local file the Host would read or write, which is the user's own.
    #[must_use]
    pub fn agent_capability(&self) -> Option<AgentCapability> {
        use AgentCapability as C;
        match self {
            Self::ChannelPins { .. }
            | Self::MessageSearch { .. }
            | Self::AttachmentInfo { .. }
            | Self::VoiceMessageAudio { .. } => Some(C::CommunitiesRead),
            Self::MessagePin { .. }
            | Self::MessageUnpin { .. }
            | Self::AttachmentPin { .. }
            | Self::VoiceMessageSend { .. } => Some(C::CommunitiesWrite),
            Self::AttachmentUpload { .. } | Self::AttachmentDownload { .. } => None,
        }
    }
}
