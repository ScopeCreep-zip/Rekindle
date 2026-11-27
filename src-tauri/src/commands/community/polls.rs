//! Polls (plan E3.1 E3), answered by the Host.

use rekindle_ipc::protocol::RichRequest;
use rekindle_ipc::protocol::{CommunityRequest, IpcRequest};
use rekindle_types::dto::{CommandError, Done};
use tauri::State;

use crate::state::SharedState;

host_command! {
    state;
    /// Attach a poll to our message (`CommunityRequest::PollCreate`).
    pub async fn create_poll(
        state: State<'_, SharedState>,
        community_id: String,
        channel_id: String,
        message_id: String,
        question: String,
        answers: Vec<String>,
        multi_select: bool,
        duration_seconds: Option<u64>,
    ) -> Result<Done<String>, CommandError> {
        IpcRequest::Community(CommunityRequest::Rich(RichRequest::PollCreate {
            community: community_id,
            channel_id,
            message_id,
            question,
            answers,
            multi_select,
            duration_seconds,
        }))
    }
}

host_command! {
    state;
    /// Vote in a poll (`CommunityRequest::PollVote`).
    pub async fn vote_poll(
        state: State<'_, SharedState>,
        community_id: String,
        channel_id: String,
        poll_id: String,
        selected_answers: Vec<u8>,
    ) -> Result<Done<serde_json::Value>, CommandError> {
        IpcRequest::Community(CommunityRequest::Rich(RichRequest::PollVote {
            community: community_id,
            channel_id,
            poll_id,
            selected_answers,
        }))
    }
}

host_command! {
    state;
    /// Close a poll (`CommunityRequest::PollClose`).
    pub async fn close_poll(
        state: State<'_, SharedState>,
        community_id: String,
        channel_id: String,
        poll_id: String,
    ) -> Result<Done<serde_json::Value>, CommandError> {
        IpcRequest::Community(CommunityRequest::Rich(RichRequest::PollClose {
            community: community_id,
            channel_id,
            poll_id,
        }))
    }
}

host_command! {
    state;
    /// A poll's counts (`CommunityRequest::PollResults`).
    pub async fn get_poll_results(
        state: State<'_, SharedState>,
        community_id: String,
        channel_id: String,
        poll_id: String,
    ) -> Result<Done<Vec<u32>>, CommandError> {
        IpcRequest::Community(CommunityRequest::Rich(RichRequest::PollResults {
            community: community_id,
            channel_id,
            poll_id,
        }))
    }
}
