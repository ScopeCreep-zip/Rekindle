//! Reaction, thread and poll requests (plan E3.1 E3), answered by the
//! Host's channel adapter on both modes.

use rekindle_channel::error::ChannelError;
use rekindle_ipc::protocol::IpcResponse;
use rekindle_ipc::protocol::RichRequest;

use crate::daemon::DaemonState;
use crate::host::adapters::channel::rich::{NewPoll, NewThread};
use crate::host::adapters::channel::HostChannel;

use super::{state_error, DaemonContext};

/// A refusal or failure of `what` as the IPC reports it.
fn refused(e: &ChannelError, what: &str) -> IpcResponse {
    let code = match e {
        ChannelError::PermissionDenied(_) => 403,
        ChannelError::ChannelNotFound(_) | ChannelError::CommunityNotFound(_) => 404,
        ChannelError::InvalidId(_) | ChannelError::BodyTooLarge { .. } => 400,
        ChannelError::SlowmodeActive { .. } => 429,
        _ => 500,
    };
    IpcResponse::error(code, format!("{what}: {e}"))
}

fn answer<T: serde::Serialize>(result: Result<T, ChannelError>, what: &str) -> IpcResponse {
    match result {
        Ok(value) => IpcResponse::ok(&value),
        Err(e) => refused(&e, what),
    }
}

fn community_of(request: &RichRequest) -> &str {
    match request {
        RichRequest::ReactionAdd { community, .. }
        | RichRequest::ReactionRemove { community, .. }
        | RichRequest::ThreadCreate { community, .. }
        | RichRequest::ThreadArchive { community, .. }
        | RichRequest::ThreadUnarchive { community, .. }
        | RichRequest::ThreadList { community, .. }
        | RichRequest::ThreadListActive { community, .. }
        | RichRequest::ThreadMessages { community, .. }
        | RichRequest::ThreadSend { community, .. }
        | RichRequest::PollCreate { community, .. }
        | RichRequest::PollVote { community, .. }
        | RichRequest::PollClose { community, .. }
        | RichRequest::PollResults { community, .. } => community,
    }
}

/// A reaction, thread or poll request.
pub(crate) async fn handle(
    ctx: &DaemonContext,
    state: DaemonState,
    request: RichRequest,
) -> IpcResponse {
    let read = matches!(
        request,
        RichRequest::ThreadList { .. }
            | RichRequest::ThreadListActive { .. }
            | RichRequest::ThreadMessages { .. }
            | RichRequest::PollResults { .. }
    );
    if read && !state.can_query() {
        return state_error(state, "query");
    }
    if !read && !state.can_write() {
        return state_error(state, "write");
    }
    let key = match ctx.resolve_community(community_of(&request)) {
        Ok(m) => m.governance_key,
        Err(e) => return e,
    };
    let host = HostChannel::new(ctx);
    match request {
        RichRequest::ReactionAdd {
            channel_id,
            message_id,
            emoji,
            ..
        } => answer(
            host.react(&key, &channel_id, &message_id, &emoji, true)
                .await,
            "reaction add",
        ),
        RichRequest::ReactionRemove {
            channel_id,
            message_id,
            emoji,
            ..
        } => answer(
            host.react(&key, &channel_id, &message_id, &emoji, false)
                .await,
            "reaction remove",
        ),
        RichRequest::ThreadCreate {
            channel_id,
            name,
            starter_message_id,
            forum_tag,
            auto_archive_seconds,
            ..
        } => {
            let thread = NewThread {
                channel_id: &channel_id,
                name: &name,
                starter_message_id: &starter_message_id,
                forum_tag,
                auto_archive_seconds,
            };
            answer(host.create_thread(&key, thread).await, "thread create")
        }
        RichRequest::ThreadArchive { thread_id, .. } => answer(
            host.archive_thread(&key, &thread_id).await,
            "thread archive",
        ),
        RichRequest::ThreadUnarchive { thread_id, .. } => answer(
            host.unarchive_thread(&key, &thread_id).await,
            "thread unarchive",
        ),
        RichRequest::ThreadList { channel_id, .. } => {
            answer(host.threads(&key, &channel_id, false).await, "thread list")
        }
        RichRequest::ThreadListActive { channel_id, .. } => answer(
            host.threads(&key, &channel_id, true).await,
            "active threads",
        ),
        RichRequest::ThreadMessages {
            thread_id,
            limit,
            before_timestamp,
            ..
        } => answer(
            host.thread_messages(&key, &thread_id, limit, before_timestamp)
                .await,
            "thread messages",
        ),
        RichRequest::ThreadSend {
            thread_id, body, ..
        } => answer(
            host.send_thread_message(&key, &thread_id, &body).await,
            "thread send",
        ),
        RichRequest::PollCreate {
            channel_id,
            message_id,
            question,
            answers,
            multi_select,
            duration_seconds,
            ..
        } => {
            let poll = NewPoll {
                channel_id: &channel_id,
                message_id: &message_id,
                question: &question,
                answers,
                multi_select,
                duration_seconds,
            };
            answer(host.create_poll(&key, poll).await, "poll create")
        }
        RichRequest::PollVote {
            channel_id,
            poll_id,
            selected_answers,
            ..
        } => answer(
            host.vote_poll(&key, &channel_id, &poll_id, selected_answers)
                .await,
            "poll vote",
        ),
        RichRequest::PollClose {
            channel_id,
            poll_id,
            ..
        } => answer(
            host.close_poll(&key, &channel_id, &poll_id).await,
            "poll close",
        ),
        RichRequest::PollResults {
            channel_id,
            poll_id,
            ..
        } => answer(
            host.poll_results(&key, &channel_id, &poll_id).await,
            "poll results",
        ),
    }
}
