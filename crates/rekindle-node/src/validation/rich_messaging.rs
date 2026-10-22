//! The reaction, thread and poll requests' fields (plan E3.1 E3).

use rekindle_ipc::protocol::IpcResponse;
use rekindle_ipc::protocol::RichRequest;

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

pub(super) fn check(request: &RichRequest) -> Checked {
    match request {
        RichRequest::ReactionAdd {
            community: c,
            channel_id,
            message_id: m,
            emoji,
        }
        | RichRequest::ReactionRemove {
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
        RichRequest::ThreadCreate {
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
        RichRequest::ThreadArchive {
            community: c,
            thread_id,
        }
        | RichRequest::ThreadUnarchive {
            community: c,
            thread_id,
        } => {
            community(c)?;
            hex16(thread_id, "thread id")
        }
        RichRequest::ThreadList {
            community: c,
            channel_id,
        }
        | RichRequest::ThreadListActive {
            community: c,
            channel_id,
        } => {
            community(c)?;
            channel(channel_id)
        }
        RichRequest::ThreadMessages {
            community: c,
            thread_id,
            limit,
            ..
        } => {
            community(c)?;
            hex16(thread_id, "thread id")?;
            page(*limit)
        }
        RichRequest::ThreadSend {
            community: c,
            thread_id,
            body,
        } => {
            community(c)?;
            hex16(thread_id, "thread id")?;
            message_body(body)
        }
        RichRequest::PollCreate {
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
        RichRequest::PollVote {
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
        RichRequest::PollClose {
            community: c,
            channel_id,
            poll_id,
        }
        | RichRequest::PollResults {
            community: c,
            channel_id,
            poll_id,
        } => {
            community(c)?;
            channel(channel_id)?;
            hex16(poll_id, "poll id")
        }
    }
}
