//! Friends, invites, blocks, groups, session resets and verification are
//! Host calls (plans E2.1, E2.3): the Host's friend operations run them on
//! both hosts, and the windows learn their effects from
//! `SubscriptionEvent::Friend` (verification from its own answer).

use rekindle_ipc::protocol::IpcRequest;
use rekindle_types::dto::friends::{
    BlockedUser, Friend, InviteCreated, OutgoingInvite, PendingRequest, SafetyNumberView,
};
use rekindle_types::dto::{CommandError, Done};
use tauri::State;

use crate::host_call;
use crate::state::SharedState;

/// What a friend action answers when it ran.
type Ran = Done<serde_json::Value>;

/// The requests waiting for our answer (`IpcRequest::FriendRequests`).
#[tauri::command]
pub async fn get_pending_requests(
    state: State<'_, SharedState>,
) -> Result<Done<Vec<PendingRequest>>, CommandError> {
    host_call::call(&state, IpcRequest::FriendRequests).await
}

/// Add a friend by public key (`IpcRequest::FriendAdd`).
#[tauri::command]
pub async fn add_friend(
    public_key: String,
    display_name: String,
    message: String,
    state: State<'_, SharedState>,
) -> Result<Ran, CommandError> {
    host_call::call(
        &state,
        IpcRequest::FriendAdd {
            public_key,
            display_name,
            message,
        },
    )
    .await
}

/// Remove a friend (`IpcRequest::FriendRemove`).
#[tauri::command]
pub async fn remove_friend(
    public_key: String,
    state: State<'_, SharedState>,
) -> Result<Ran, CommandError> {
    host_call::call(&state, IpcRequest::FriendRemove { public_key }).await
}

/// Accept a pending request (`IpcRequest::FriendAccept`).
#[tauri::command]
pub async fn accept_request(
    public_key: String,
    display_name: String,
    state: State<'_, SharedState>,
) -> Result<Ran, CommandError> {
    host_call::call(
        &state,
        IpcRequest::FriendAccept {
            public_key,
            display_name,
        },
    )
    .await
}

/// The buddy list (`IpcRequest::FriendList`).
#[tauri::command]
pub async fn get_friends(state: State<'_, SharedState>) -> Result<Done<Vec<Friend>>, CommandError> {
    host_call::call(&state, IpcRequest::FriendList).await
}

/// Reject a pending request (`IpcRequest::FriendReject`).
#[tauri::command]
pub async fn reject_request(
    public_key: String,
    state: State<'_, SharedState>,
) -> Result<Ran, CommandError> {
    host_call::call(&state, IpcRequest::FriendReject { public_key }).await
}

/// Cancel the request we sent (`IpcRequest::FriendCancelRequest`).
#[tauri::command]
pub async fn cancel_request(
    public_key: String,
    state: State<'_, SharedState>,
) -> Result<Ran, CommandError> {
    host_call::call(&state, IpcRequest::FriendCancelRequest { public_key }).await
}

/// Retry the secure session that did not open with a friend
/// (`IpcRequest::FriendRetryHandshake`, plan E2.4 B7).
#[tauri::command]
pub async fn retry_handshake(
    public_key: String,
    state: State<'_, SharedState>,
) -> Result<Ran, CommandError> {
    host_call::call(&state, IpcRequest::FriendRetryHandshake { public_key }).await
}

/// Reset our secure session with a friend (`IpcRequest::SessionResetStart`,
/// TrustChanging): the review shows whose session it is; the friend
/// compares safety numbers before accepting.
#[tauri::command]
pub async fn reset_signal_session(
    peer_public_key: String,
    state: State<'_, SharedState>,
) -> Result<Ran, CommandError> {
    host_call::call(
        &state,
        IpcRequest::SessionResetStart {
            public_key: peer_public_key,
        },
    )
    .await
}

/// Our safety number with a friend, its QR, and whether we verified their
/// key (`IpcRequest::IdentitySafetyNumber`, plan E2.3).
#[tauri::command]
pub async fn get_safety_number(
    public_key: String,
    state: State<'_, SharedState>,
) -> Result<Done<SafetyNumberView>, CommandError> {
    host_call::call(
        &state,
        IpcRequest::IdentitySafetyNumber { peer: public_key },
    )
    .await
}

/// Mark a friend's key verified after comparing the safety number
/// (`IpcRequest::IdentityMarkVerified`, TrustChanging: the review states
/// the number).
#[tauri::command]
pub async fn mark_identity_verified(
    public_key: String,
    state: State<'_, SharedState>,
) -> Result<Ran, CommandError> {
    host_call::call(
        &state,
        IpcRequest::IdentityMarkVerified { peer: public_key },
    )
    .await
}

/// Withdraw a friend's verification
/// (`IpcRequest::IdentityClearVerification`, TrustChanging).
#[tauri::command]
pub async fn clear_identity_verification(
    public_key: String,
    state: State<'_, SharedState>,
) -> Result<Ran, CommandError> {
    host_call::call(
        &state,
        IpcRequest::IdentityClearVerification { peer: public_key },
    )
    .await
}

/// Create a friend group (`IpcRequest::FriendGroupCreate`).
#[tauri::command]
pub async fn create_friend_group(
    name: String,
    state: State<'_, SharedState>,
) -> Result<Ran, CommandError> {
    host_call::call(&state, IpcRequest::FriendGroupCreate { name }).await
}

/// Set (or clear) our alias for one friend (`IpcRequest::FriendSetNickname`),
/// distinct from `set_nickname`, our own display name.
#[tauri::command]
pub async fn set_friend_nickname(
    public_key: String,
    nickname: Option<String>,
    state: State<'_, SharedState>,
) -> Result<Ran, CommandError> {
    host_call::call(
        &state,
        IpcRequest::FriendSetNickname {
            public_key,
            nickname,
        },
    )
    .await
}

/// Rename a friend group (`IpcRequest::FriendGroupRename`).
#[tauri::command]
pub async fn rename_friend_group(
    group_id: i64,
    name: String,
    state: State<'_, SharedState>,
) -> Result<Ran, CommandError> {
    host_call::call(&state, IpcRequest::FriendGroupRename { group_id, name }).await
}

/// Delete a friend group; its friends fall back to no group
/// (`IpcRequest::FriendGroupDelete`).
#[tauri::command]
pub async fn delete_friend_group(
    group_id: i64,
    state: State<'_, SharedState>,
) -> Result<Ran, CommandError> {
    host_call::call(&state, IpcRequest::FriendGroupDelete { group_id }).await
}

/// Move a friend into a group, or out of any with `None`
/// (`IpcRequest::FriendGroupMove`).
#[tauri::command]
pub async fn move_friend_to_group(
    public_key: String,
    group_id: Option<i64>,
    state: State<'_, SharedState>,
) -> Result<Ran, CommandError> {
    host_call::call(
        &state,
        IpcRequest::FriendGroupMove {
            public_key,
            group_id,
        },
    )
    .await
}

/// Make a tracked invite link (`IpcRequest::FriendInviteCreate`).
#[tauri::command]
pub async fn generate_invite(
    state: State<'_, SharedState>,
) -> Result<Done<InviteCreated>, CommandError> {
    host_call::call(&state, IpcRequest::FriendInviteCreate).await
}

/// Add the friend a pasted invite link names
/// (`IpcRequest::FriendAddFromInvite`, reviewed with their fingerprint).
#[tauri::command]
pub async fn add_friend_from_invite(
    invite_string: String,
    state: State<'_, SharedState>,
) -> Result<Ran, CommandError> {
    host_call::call(
        &state,
        IpcRequest::FriendAddFromInvite {
            invite: invite_string,
        },
    )
    .await
}

/// Block anyone by public key (`IpcRequest::FriendBlock`): our profile
/// moves to a new key.
#[tauri::command]
pub async fn block_user(
    public_key: String,
    display_name: Option<String>,
    state: State<'_, SharedState>,
) -> Result<Ran, CommandError> {
    host_call::call(
        &state,
        IpcRequest::FriendBlock {
            public_key,
            display_name,
        },
    )
    .await
}

/// Unblock a user (`IpcRequest::FriendUnblock`); they are not re-added.
#[tauri::command]
pub async fn unblock_user(
    public_key: String,
    state: State<'_, SharedState>,
) -> Result<Ran, CommandError> {
    host_call::call(&state, IpcRequest::FriendUnblock { public_key }).await
}

/// Everyone we blocked (`IpcRequest::FriendBlockedList`).
#[tauri::command]
pub async fn get_blocked_users(
    state: State<'_, SharedState>,
) -> Result<Done<Vec<BlockedUser>>, CommandError> {
    host_call::call(&state, IpcRequest::FriendBlockedList).await
}

/// Cancel an open invite (`IpcRequest::FriendInviteCancel`).
#[tauri::command]
pub async fn cancel_invite(
    invite_id: String,
    state: State<'_, SharedState>,
) -> Result<Ran, CommandError> {
    host_call::call(&state, IpcRequest::FriendInviteCancel { invite_id }).await
}

/// Our open invites (`IpcRequest::FriendInviteList`).
#[tauri::command]
pub async fn get_outgoing_invites(
    state: State<'_, SharedState>,
) -> Result<Done<Vec<OutgoingInvite>>, CommandError> {
    host_call::call(&state, IpcRequest::FriendInviteList).await
}
