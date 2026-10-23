//! Pins, search and attachments (plan E3.1 E3b, E3d): a search query, a
//! local path and a voice message's bytes show their length only.

use crate::protocol::PinRequest;

impl std::fmt::Debug for PinRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ChannelPins {
                community,
                channel_id,
            } => f
                .debug_struct("ChannelPins")
                .field("community", community)
                .field("channel_id", channel_id)
                .finish(),
            Self::MessagePin {
                community,
                channel_id,
                message_id,
            }
            | Self::MessageUnpin {
                community,
                channel_id,
                message_id,
            } => f
                .debug_struct(self.name())
                .field("community", community)
                .field("channel_id", channel_id)
                .field("message_id", message_id)
                .finish(),
            Self::AttachmentPin {
                community,
                attachment_id,
                pinned,
            } => f
                .debug_struct("AttachmentPin")
                .field("community", community)
                .field("attachment_id", attachment_id)
                .field("pinned", pinned)
                .finish(),
            Self::AttachmentUpload {
                community,
                channel_id,
                path,
            } => f
                .debug_struct("AttachmentUpload")
                .field("community", community)
                .field("channel_id", channel_id)
                .field("path", &path.len())
                .finish(),
            Self::AttachmentDownload {
                community,
                channel_id,
                attachment_id,
                path,
            } => f
                .debug_struct("AttachmentDownload")
                .field("community", community)
                .field("channel_id", channel_id)
                .field("attachment_id", attachment_id)
                .field("path", &path.len())
                .finish(),
            Self::AttachmentInfo {
                community,
                channel_id,
                attachment_id,
            }
            | Self::VoiceMessageAudio {
                community,
                channel_id,
                attachment_id,
            } => f
                .debug_struct(self.name())
                .field("community", community)
                .field("channel_id", channel_id)
                .field("attachment_id", attachment_id)
                .finish(),
            Self::VoiceMessageSend {
                community,
                channel_id,
                opus,
                duration_ms,
                waveform,
            } => f
                .debug_struct("VoiceMessageSend")
                .field("community", community)
                .field("channel_id", channel_id)
                .field("opus", &opus.len())
                .field("duration_ms", duration_ms)
                .field("waveform", &waveform.len())
                .finish(),
            Self::MessageSearch { request } => f
                .debug_struct("MessageSearch")
                .field("query", &request.query.len())
                .field("limit", &request.limit)
                .finish_non_exhaustive(),
        }
    }
}
