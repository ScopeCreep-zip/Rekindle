//! Polls (plan E3.1 E3), answered by the Host.

use rekindle_ipc::protocol::RichRequest;
use rekindle_ipc::protocol::{CommunityRequest, IpcRequest};
use rekindle_types::dto::{CommandError, Done};
use tauri::State;

use crate::host_call;
use crate::state::SharedState;

/// Attach a poll to our message (`CommunityRequest::PollCreate`).
#[tauri::command]
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
    host_call::call(
        &state,
        IpcRequest::Community(CommunityRequest::Rich(RichRequest::PollCreate {
            community: community_id,
            channel_id,
            message_id,
            question,
            answers,
            multi_select,
            duration_seconds,
        })),
    )
    .await
}

/// Vote in a poll (`CommunityRequest::PollVote`).
#[tauri::command]
pub async fn vote_poll(
    state: State<'_, SharedState>,
    community_id: String,
    channel_id: String,
    poll_id: String,
    selected_answers: Vec<u8>,
) -> Result<Done<serde_json::Value>, CommandError> {
    host_call::call(
        &state,
        IpcRequest::Community(CommunityRequest::Rich(RichRequest::PollVote {
            community: community_id,
            channel_id,
            poll_id,
            selected_answers,
        })),
    )
    .await
}

/// Close a poll (`CommunityRequest::PollClose`).
#[tauri::command]
pub async fn close_poll(
    state: State<'_, SharedState>,
    community_id: String,
    channel_id: String,
    poll_id: String,
) -> Result<Done<serde_json::Value>, CommandError> {
    host_call::call(
        &state,
        IpcRequest::Community(CommunityRequest::Rich(RichRequest::PollClose {
            community: community_id,
            channel_id,
            poll_id,
        })),
    )
    .await
}

/// A poll's counts (`CommunityRequest::PollResults`).
#[tauri::command]
pub async fn get_poll_results(
    state: State<'_, SharedState>,
    community_id: String,
    channel_id: String,
    poll_id: String,
) -> Result<Done<Vec<u32>>, CommandError> {
    host_call::call(
        &state,
        IpcRequest::Community(CommunityRequest::Rich(RichRequest::PollResults {
            community: community_id,
            channel_id,
            poll_id,
        })),
    )
    .await
}
