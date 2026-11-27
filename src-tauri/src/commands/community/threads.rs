//! Threads (plan E3.1 E3), answered by the Host.

use rekindle_ipc::protocol::RichRequest;
use rekindle_ipc::protocol::{CommunityRequest, IpcRequest};
use rekindle_types::dto::{CommandError, Done};
use tauri::State;

use crate::state::SharedState;
use rekindle_types::dto::channel_message::ChannelMessageView;
use rekindle_types::thread::ThreadInfo;

host_command! {
    state;
    /// Open a thread off a channel message (`CommunityRequest::ThreadCreate`).
    pub async fn create_thread(
        state: State<'_, SharedState>,
        community_id: String,
        channel_id: String,
        name: String,
        starter_message_id: String,
        forum_tag: Option<String>,
        auto_archive_seconds: Option<u64>,
    ) -> Result<Done<String>, CommandError> {
        IpcRequest::Community(CommunityRequest::Rich(RichRequest::ThreadCreate {
            community: community_id,
            channel_id,
            name,
            starter_message_id,
            forum_tag,
            auto_archive_seconds,
        }))
    }
}

host_command! {
    state;
    /// A channel's threads (`CommunityRequest::ThreadList`).
    pub async fn get_channel_threads(
        state: State<'_, SharedState>,
        community_id: String,
        channel_id: String,
    ) -> Result<Done<Vec<ThreadInfo>>, CommandError> {
        IpcRequest::Community(CommunityRequest::Rich(RichRequest::ThreadList {
            community: community_id,
            channel_id,
        }))
    }
}

host_command! {
    state;
    /// A channel's open threads (`CommunityRequest::ThreadListActive`).
    pub async fn get_active_threads(
        state: State<'_, SharedState>,
        community_id: String,
        channel_id: String,
    ) -> Result<Done<Vec<ThreadInfo>>, CommandError> {
        IpcRequest::Community(CommunityRequest::Rich(RichRequest::ThreadListActive {
            community: community_id,
            channel_id,
        }))
    }
}

host_command! {
    state;
    /// Post to a thread (`CommunityRequest::ThreadSend`).
    pub async fn send_thread_message(
        state: State<'_, SharedState>,
        community_id: String,
        thread_id: String,
        body: String,
    ) -> Result<Done<serde_json::Value>, CommandError> {
        IpcRequest::Community(CommunityRequest::Rich(RichRequest::ThreadSend {
            community: community_id,
            thread_id,
            body,
        }))
    }
}

host_command! {
    state;
    /// A page of a thread's messages (`CommunityRequest::ThreadMessages`).
    pub async fn get_thread_messages(
        state: State<'_, SharedState>,
        community_id: String,
        thread_id: String,
        limit: u32,
        before_timestamp: Option<u64>,
    ) -> Result<Done<Vec<ChannelMessageView>>, CommandError> {
        IpcRequest::Community(CommunityRequest::Rich(RichRequest::ThreadMessages {
            community: community_id,
            thread_id,
            limit,
            before_timestamp,
        }))
    }
}

host_command! {
    state;
    /// Archive a thread (`CommunityRequest::ThreadArchive`).
    pub async fn archive_thread(
        state: State<'_, SharedState>,
        community_id: String,
        thread_id: String,
    ) -> Result<Done<serde_json::Value>, CommandError> {
        IpcRequest::Community(CommunityRequest::Rich(RichRequest::ThreadArchive {
            community: community_id,
            thread_id,
        }))
    }
}

host_command! {
    state;
    /// Reopen a thread (`CommunityRequest::ThreadUnarchive`).
    pub async fn unarchive_thread(
        state: State<'_, SharedState>,
        community_id: String,
        thread_id: String,
    ) -> Result<Done<serde_json::Value>, CommandError> {
        IpcRequest::Community(CommunityRequest::Rich(RichRequest::ThreadUnarchive {
            community: community_id,
            thread_id,
        }))
    }
}
