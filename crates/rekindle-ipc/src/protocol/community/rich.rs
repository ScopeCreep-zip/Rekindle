//! The reactions, threads and polls (plan E3.1), carried as `CommunityRequest::Rich`.

use serde::{Deserialize, Serialize};

use crate::message::SecurityLevel;
use crate::protocol::{AgentCapability, Lane};

/// A reactions, threads and polls request.
#[derive(Clone, Serialize, Deserialize)]
pub enum RichRequest {
    /// React to a channel message with an expression.
    ReactionAdd {
        community: String,
        channel_id: String,
        message_id: String,
        emoji: String,
    },

    /// Take our reaction off a channel message.
    ReactionRemove {
        community: String,
        channel_id: String,
        message_id: String,
        emoji: String,
    },

    /// Open a thread off a channel message; answered with its id.
    ThreadCreate {
        community: String,
        channel_id: String,
        name: String,
        starter_message_id: String,
        forum_tag: Option<String>,
        auto_archive_seconds: Option<u64>,
    },

    /// Archive a thread.
    ThreadArchive {
        community: String,
        thread_id: String,
    },

    /// Reopen an archived thread.
    ThreadUnarchive {
        community: String,
        thread_id: String,
    },

    /// A channel's threads, the most recently active first.
    ThreadList {
        community: String,
        channel_id: String,
    },

    /// A channel's threads that are not archived.
    ThreadListActive {
        community: String,
        channel_id: String,
    },

    /// A page of a thread's messages, oldest first, before
    /// `before_timestamp` (seconds) when given.
    ThreadMessages {
        community: String,
        thread_id: String,
        limit: u32,
        before_timestamp: Option<u64>,
    },

    /// Post `body` to a thread.
    ThreadSend {
        community: String,
        thread_id: String,
        body: String,
    },

    /// Attach a poll to our message; answered with the poll's id.
    PollCreate {
        community: String,
        channel_id: String,
        message_id: String,
        question: String,
        answers: Vec<String>,
        multi_select: bool,
        duration_seconds: Option<u64>,
    },

    /// Vote in a poll (our latest vote counts).
    PollVote {
        community: String,
        channel_id: String,
        poll_id: String,
        selected_answers: Vec<u8>,
    },

    /// Close a poll (its author, or a member who manages messages).
    PollClose {
        community: String,
        channel_id: String,
        poll_id: String,
    },

    /// A poll's vote count per answer.
    PollResults {
        community: String,
        channel_id: String,
        poll_id: String,
    },
}

impl RichRequest {
    /// The variant name, without its fields.
    #[must_use]
    pub fn name(&self) -> &'static str {
        self.class().0
    }

    pub(crate) fn class(&self) -> (&'static str, Lane, SecurityLevel) {
        use Lane::{Query, Write};
        use SecurityLevel::Authenticated;
        match self {
            Self::ReactionAdd { .. } => ("ReactionAdd", Write, Authenticated),
            Self::ReactionRemove { .. } => ("ReactionRemove", Write, Authenticated),
            Self::ThreadCreate { .. } => ("ThreadCreate", Write, Authenticated),
            Self::ThreadArchive { .. } => ("ThreadArchive", Write, Authenticated),
            Self::ThreadUnarchive { .. } => ("ThreadUnarchive", Write, Authenticated),
            Self::ThreadList { .. } => ("ThreadList", Query, Authenticated),
            Self::ThreadListActive { .. } => ("ThreadListActive", Query, Authenticated),
            Self::ThreadMessages { .. } => ("ThreadMessages", Query, Authenticated),
            Self::ThreadSend { .. } => ("ThreadSend", Write, Authenticated),
            Self::PollCreate { .. } => ("PollCreate", Write, Authenticated),
            Self::PollVote { .. } => ("PollVote", Write, Authenticated),
            Self::PollClose { .. } => ("PollClose", Write, Authenticated),
            Self::PollResults { .. } => ("PollResults", Query, Authenticated),
        }
    }

    /// The capability an `Agent`-level sender needs for this request, which every
    /// request of this domain has.
    #[must_use]
    pub fn agent_capability(&self) -> AgentCapability {
        use AgentCapability as C;
        match self {
            Self::ThreadList { .. }
            | Self::ThreadListActive { .. }
            | Self::ThreadMessages { .. }
            | Self::PollResults { .. } => C::ChannelsHistory,
            Self::ReactionAdd { .. }
            | Self::ReactionRemove { .. }
            | Self::ThreadCreate { .. }
            | Self::ThreadArchive { .. }
            | Self::ThreadUnarchive { .. }
            | Self::ThreadSend { .. }
            | Self::PollCreate { .. }
            | Self::PollVote { .. }
            | Self::PollClose { .. } => C::ChatWrite,
        }
    }
}
