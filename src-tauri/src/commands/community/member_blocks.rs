//! A community block by pseudonym (plan E3.1 F3, D26), answered by the Host.

use rekindle_ipc::protocol::MemberRequest;
use rekindle_ipc::protocol::{CommunityRequest, IpcRequest};
use rekindle_types::dto::{CommandError, Done};
use tauri::State;

use crate::state::SharedState;

host_command! {
    state;
    pub async fn block_community_member(
        state: State<'_, SharedState>,
        community_id: String,
        pseudonym_key: String,
    ) -> Result<Done<()>, CommandError> {
        IpcRequest::Community(CommunityRequest::Members(MemberRequest::MemberBlock {
            community: community_id,
            target_pseudonym: pseudonym_key,
        }))
    }
}

host_command! {
    state;
    pub async fn unblock_community_member(
        state: State<'_, SharedState>,
        community_id: String,
        pseudonym_key: String,
    ) -> Result<Done<()>, CommandError> {
        IpcRequest::Community(CommunityRequest::Members(MemberRequest::MemberUnblock {
            community: community_id,
            target_pseudonym: pseudonym_key,
        }))
    }
}

host_command! {
    state;
    pub async fn list_community_blocks(
        state: State<'_, SharedState>,
        community_id: String,
    ) -> Result<Done<Vec<String>>, CommandError> {
        IpcRequest::Community(CommunityRequest::Members(MemberRequest::MemberBlockList {
            community: community_id,
        }))
    }
}
