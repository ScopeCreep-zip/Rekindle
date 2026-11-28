//! Entry helpers.

use super::types::{ChannelMessage, ChannelRecordEntry};

pub fn message_from_entry(entry: &ChannelRecordEntry) -> Option<&ChannelMessage> {
    match entry {
        ChannelRecordEntry::Message(message) => Some(message),
        ChannelRecordEntry::Forward(_)
        | ChannelRecordEntry::AttachmentCached(_)
        | ChannelRecordEntry::Reaction(_)
        | ChannelRecordEntry::PollCreate(_)
        | ChannelRecordEntry::PollVote(_)
        | ChannelRecordEntry::PollClose(_)
        | ChannelRecordEntry::HandRaise(_)
        | ChannelRecordEntry::Edit(_)
        | ChannelRecordEntry::Delete(_) => None,
    }
}
