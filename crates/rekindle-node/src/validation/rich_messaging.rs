//! The reaction, thread and poll requests' fields (plan E3.1 E3).

use rekindle_ipc::protocol::{CommunityRequest, IpcResponse};

use super::{channel, community, hex16, message_body, name, opt, page, text, Checked};

/// Longest message id a reaction or poll names.
const MAX_MESSAGE_ID_LEN: usize = 128;
/// Longest expression a reaction carries.
const MAX_EXPRESSION_LEN: usize = 64;
/// Longest forum tag.
const MAX_FORUM_TAG_LEN: usize = 64;

fn message_id(id: &str) -> Checked {
    text(id, MAX_MESSAGE_ID_LEN, "message id")
}

fn refused(e: &rekindle_channel::error::ChannelError) -> IpcResponse {
    IpcResponse::error(400, e.to_string())
}

/// Whether `request` is a reaction, thread or poll request.
pub(super) fn is_rich(request: &CommunityRequest) -> bool {
    matches!(
        request,
        CommunityRequest::ReactionAdd { .. }
            | CommunityRequest::ReactionRemove { .. }
            | CommunityRequest::ThreadCreate { .. }
            | CommunityRequest::ThreadArchive { .. }
            | CommunityRequest::ThreadUnarchive { .. }
            | CommunityRequest::ThreadList { .. }
            | CommunityRequest::ThreadListActive { .. }
            | CommunityRequest::ThreadMessages { .. }
            | CommunityRequest::ThreadSend { .. }
            | CommunityRequest::PollCreate { .. }
            | CommunityRequest::PollVote { .. }
            | CommunityRequest::PollClose { .. }
            | CommunityRequest::PollResults { .. }
    )
}

pub(super) fn check(request: &CommunityRequest) -> Checked {
    match request {
        CommunityRequest::ReactionAdd {
            community: c,
            channel_id,
            message_id: m,
            emoji,
        }
        | CommunityRequest::ReactionRemove {
            community: c,
            channel_id,
            message_id: m,
            emoji,
        } => {
            community(c)?;
            channel(channel_id)?;
            message_id(m)?;
            text(emoji, MAX_EXPRESSION_LEN, "expression")
        }
        CommunityRequest::ThreadCreate {
            community: c,
            channel_id,
            name: thread_name,
            starter_message_id,
            forum_tag,
            auto_archive_seconds,
        } => {
            community(c)?;
            channel(channel_id)?;
            name(thread_name, "thread")?;
            message_id(starter_message_id)?;
            opt(forum_tag.as_deref(), |tag| {
                text(tag, MAX_FORUM_TAG_LEN, "forum tag")
            })?;
            opt(auto_archive_seconds.as_ref(), |secs| {
                rekindle_channel::validate_auto_archive_seconds(*secs)
                    .map(drop)
                    .map_err(|e| refused(&e))
            })
        }
        CommunityRequest::ThreadArchive {
            community: c,
            thread_id,
        }
        | CommunityRequest::ThreadUnarchive {
            community: c,
            thread_id,
        } => {
            community(c)?;
            hex16(thread_id, "thread id")
        }
        CommunityRequest::ThreadList {
            community: c,
            channel_id,
        }
        | CommunityRequest::ThreadListActive {
            community: c,
            channel_id,
        } => {
            community(c)?;
            channel(channel_id)
        }
        CommunityRequest::ThreadMessages {
            community: c,
            thread_id,
            limit,
            ..
        } => {
            community(c)?;
            hex16(thread_id, "thread id")?;
            page(*limit)
        }
        CommunityRequest::ThreadSend {
            community: c,
            thread_id,
            body,
        } => {
            community(c)?;
            hex16(thread_id, "thread id")?;
            message_body(body)
        }
        CommunityRequest::PollCreate {
            community: c,
            channel_id,
            message_id: m,
            question,
            answers,
            ..
        } => {
            community(c)?;
            channel(channel_id)?;
            message_id(m)?;
            rekindle_channel::validate_poll_create(question, answers).map_err(|e| refused(&e))
        }
        CommunityRequest::PollVote {
            community: c,
            channel_id,
            poll_id,
            selected_answers,
        } => {
            community(c)?;
            channel(channel_id)?;
            hex16(poll_id, "poll id")?;
            if selected_answers.is_empty() {
                return Err(IpcResponse::error(
                    400,
                    "a vote selects at least one answer",
                ));
            }
            Ok(())
        }
        CommunityRequest::PollClose {
            community: c,
            channel_id,
            poll_id,
        }
        | CommunityRequest::PollResults {
            community: c,
            channel_id,
            poll_id,
        } => {
            community(c)?;
            channel(channel_id)?;
            hex16(poll_id, "poll id")
        }
        _ => Ok(()),
    }
}
