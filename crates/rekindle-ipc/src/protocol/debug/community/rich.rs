//! `Debug` for the reaction, thread and poll requests: bodies, names,
//! questions and answers are what members wrote, so only their sizes are
//! logged.

use crate::protocol::CommunityRequest;

impl CommunityRequest {
    pub(super) fn fmt_rich(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ReactionAdd {
                community,
                channel_id,
                message_id,
                ..
            }
            | Self::ReactionRemove {
                community,
                channel_id,
                message_id,
                ..
            } => f
                .debug_struct(self.name())
                .field("community", community)
                .field("channel_id", channel_id)
                .field("message_id", message_id)
                .finish_non_exhaustive(),
            Self::ThreadCreate {
                community,
                channel_id,
                name,
                starter_message_id,
                auto_archive_seconds,
                ..
            } => f
                .debug_struct("ThreadCreate")
                .field("community", community)
                .field("channel_id", channel_id)
                .field("name_len", &name.len())
                .field("starter_message_id", starter_message_id)
                .field("auto_archive_seconds", auto_archive_seconds)
                .finish_non_exhaustive(),
            Self::ThreadArchive {
                community,
                thread_id,
            }
            | Self::ThreadUnarchive {
                community,
                thread_id,
            } => f
                .debug_struct(self.name())
                .field("community", community)
                .field("thread_id", thread_id)
                .finish(),
            Self::ThreadList {
                community,
                channel_id,
            }
            | Self::ThreadListActive {
                community,
                channel_id,
            } => f
                .debug_struct(self.name())
                .field("community", community)
                .field("channel_id", channel_id)
                .finish(),
            Self::ThreadMessages {
                community,
                thread_id,
                limit,
                before_timestamp,
            } => f
                .debug_struct("ThreadMessages")
                .field("community", community)
                .field("thread_id", thread_id)
                .field("limit", limit)
                .field("before_timestamp", before_timestamp)
                .finish(),
            Self::ThreadSend {
                community,
                thread_id,
                body,
            } => f
                .debug_struct("ThreadSend")
                .field("community", community)
                .field("thread_id", thread_id)
                .field("body_len", &body.len())
                .finish(),
            Self::PollCreate {
                community,
                channel_id,
                message_id,
                answers,
                multi_select,
                duration_seconds,
                ..
            } => f
                .debug_struct("PollCreate")
                .field("community", community)
                .field("channel_id", channel_id)
                .field("message_id", message_id)
                .field("answers", &answers.len())
                .field("multi_select", multi_select)
                .field("duration_seconds", duration_seconds)
                .finish_non_exhaustive(),
            Self::PollVote {
                community,
                channel_id,
                poll_id,
                selected_answers,
            } => f
                .debug_struct("PollVote")
                .field("community", community)
                .field("channel_id", channel_id)
                .field("poll_id", poll_id)
                .field("selected_answers", selected_answers)
                .finish(),
            Self::PollClose {
                community,
                channel_id,
                poll_id,
            }
            | Self::PollResults {
                community,
                channel_id,
                poll_id,
            } => f
                .debug_struct(self.name())
                .field("community", community)
                .field("channel_id", channel_id)
                .field("poll_id", poll_id)
                .finish(),
            other => unreachable!("{} is not a reaction, thread or poll request", other.name()),
        }
    }
}
