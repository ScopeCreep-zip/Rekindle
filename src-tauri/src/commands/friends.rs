//! Friends, invites, blocks, groups, session resets and verification are
//! Host calls (plans E2.1, E2.3): the Host's friend operations run them on
//! both hosts, and the windows learn their effects from
//! `SubscriptionEvent::Friend` (verification from its own answer).

use rekindle_ipc::protocol::IpcRequest;
use rekindle_types::dto::friends::{
    BlockedUser, Friend, FriendCodeView, InviteCreated, OutgoingInvite, PendingRequest,
    SafetyNumberView,
};
use rekindle_types::dto::{CommandError, Done};
use tauri::State;

use crate::state::SharedState;

/// What a friend action answers when it ran.
type Ran = Done<serde_json::Value>;

host_command! {
    state;
    /// The requests waiting for our answer (`IpcRequest::FriendRequests`).
    pub async fn get_pending_requests(
        state: State<'_, SharedState>,
    ) -> Result<Done<Vec<PendingRequest>>, CommandError> {
        IpcRequest::FriendRequests
    }
}

host_command! {
    state;
    /// Send a friend request to the identity a friend code names
    /// (`IpcRequest::FriendAddFromCode`).
    pub async fn add_friend_from_code(
        code: String,
        message: String,
        state: State<'_, SharedState>,
    ) -> Result<Ran, CommandError> {
        IpcRequest::FriendAddFromCode { code, message }
    }
}

host_command! {
    state;
    /// Our friend code and its QR (`IpcRequest::FriendCode`).
    pub async fn get_friend_code(
        state: State<'_, SharedState>,
    ) -> Result<Done<FriendCodeView>, CommandError> {
        IpcRequest::FriendCode
    }
}

host_command! {
    state;
    /// Mint a new friend-request token; codes already shared stop working
    /// (`IpcRequest::FriendTokenRotate`).
    pub async fn rotate_friend_token(
        state: State<'_, SharedState>,
    ) -> Result<Done<FriendCodeView>, CommandError> {
        IpcRequest::FriendTokenRotate
    }
}

host_command! {
    state;
    /// Remove a friend (`IpcRequest::FriendRemove`).
    pub async fn remove_friend(
        public_key: String,
        state: State<'_, SharedState>,
    ) -> Result<Ran, CommandError> {
        IpcRequest::FriendRemove { public_key }
    }
}

host_command! {
    state;
    /// Accept a pending request (`IpcRequest::FriendAccept`).
    pub async fn accept_request(
        public_key: String,
        state: State<'_, SharedState>,
    ) -> Result<Ran, CommandError> {
        IpcRequest::FriendAccept { public_key }
    }
}

host_command! {
    state;
    /// The buddy list (`IpcRequest::FriendList`).
    pub async fn get_friends(state: State<'_, SharedState>) -> Result<Done<Vec<Friend>>, CommandError> {
        IpcRequest::FriendList
    }
}

host_command! {
    state;
    /// Reject a pending request (`IpcRequest::FriendReject`).
    pub async fn reject_request(
        public_key: String,
        state: State<'_, SharedState>,
    ) -> Result<Ran, CommandError> {
        IpcRequest::FriendReject { public_key }
    }
}

host_command! {
    state;
    /// Cancel the request we sent (`IpcRequest::FriendCancelRequest`).
    pub async fn cancel_request(
        public_key: String,
        state: State<'_, SharedState>,
    ) -> Result<Ran, CommandError> {
        IpcRequest::FriendCancelRequest { public_key }
    }
}

host_command! {
    state;
    /// Retry the secure session that did not open with a friend
    /// (`IpcRequest::FriendRetryHandshake`, plan E2.4 B7).
    pub async fn retry_handshake(
        public_key: String,
        state: State<'_, SharedState>,
    ) -> Result<Ran, CommandError> {
        IpcRequest::FriendRetryHandshake { public_key }
    }
}

host_command! {
    state;
    /// Reset our secure session with a friend (`IpcRequest::SessionResetStart`,
    /// TrustChanging): the review shows whose session it is; the friend
    /// compares safety numbers before accepting.
    pub async fn reset_signal_session(
        peer_public_key: String,
        state: State<'_, SharedState>,
    ) -> Result<Ran, CommandError> {
        IpcRequest::SessionResetStart {
            public_key: peer_public_key,
        }
    }
}

host_command! {
    state;
    /// Our safety number with a friend, its QR, and whether we verified their
    /// key (`IpcRequest::IdentitySafetyNumber`, plan E2.3).
    pub async fn get_safety_number(
        public_key: String,
        state: State<'_, SharedState>,
    ) -> Result<Done<SafetyNumberView>, CommandError> {
        IpcRequest::IdentitySafetyNumber { peer: public_key }
    }
}

host_command! {
    state;
    /// Mark a friend's key verified after comparing the safety number
    /// (`IpcRequest::IdentityMarkVerified`, TrustChanging: the review states
    /// the number).
    pub async fn mark_identity_verified(
        public_key: String,
        state: State<'_, SharedState>,
    ) -> Result<Ran, CommandError> {
        IpcRequest::IdentityMarkVerified { peer: public_key }
    }
}

host_command! {
    state;
    /// Withdraw a friend's verification
    /// (`IpcRequest::IdentityClearVerification`, TrustChanging).
    pub async fn clear_identity_verification(
        public_key: String,
        state: State<'_, SharedState>,
    ) -> Result<Ran, CommandError> {
        IpcRequest::IdentityClearVerification { peer: public_key }
    }
}

host_command! {
    state;
    /// Create a friend group (`IpcRequest::FriendGroupCreate`).
    pub async fn create_friend_group(
        name: String,
        state: State<'_, SharedState>,
    ) -> Result<Ran, CommandError> {
        IpcRequest::FriendGroupCreate { name }
    }
}

host_command! {
    state;
    /// Set (or clear) our alias for one friend (`IpcRequest::FriendSetNickname`),
    /// distinct from `set_nickname`, our own display name.
    pub async fn set_friend_nickname(
        public_key: String,
        nickname: Option<String>,
        state: State<'_, SharedState>,
    ) -> Result<Ran, CommandError> {
        IpcRequest::FriendSetNickname {
            public_key,
            nickname,
        }
    }
}

host_command! {
    state;
    /// Rename a friend group (`IpcRequest::FriendGroupRename`).
    pub async fn rename_friend_group(
        group_id: i64,
        name: String,
        state: State<'_, SharedState>,
    ) -> Result<Ran, CommandError> {
        IpcRequest::FriendGroupRename { group_id, name }
    }
}

host_command! {
    state;
    /// Delete a friend group; its friends fall back to no group
    /// (`IpcRequest::FriendGroupDelete`).
    pub async fn delete_friend_group(
        group_id: i64,
        state: State<'_, SharedState>,
    ) -> Result<Ran, CommandError> {
        IpcRequest::FriendGroupDelete { group_id }
    }
}

host_command! {
    state;
    /// Move a friend into a group, or out of any with `None`
    /// (`IpcRequest::FriendGroupMove`).
    pub async fn move_friend_to_group(
        public_key: String,
        group_id: Option<i64>,
        state: State<'_, SharedState>,
    ) -> Result<Ran, CommandError> {
        IpcRequest::FriendGroupMove {
            public_key,
            group_id,
        }
    }
}

host_command! {
    state;
    /// Make a tracked invite link (`IpcRequest::FriendInviteCreate`).
    pub async fn generate_invite(
        state: State<'_, SharedState>,
    ) -> Result<Done<InviteCreated>, CommandError> {
        IpcRequest::FriendInviteCreate
    }
}

host_command! {
    state;
    /// Add the friend a pasted invite link names
    /// (`IpcRequest::FriendAddFromInvite`, reviewed with their fingerprint).
    pub async fn add_friend_from_invite(
        invite_string: String,
        state: State<'_, SharedState>,
    ) -> Result<Ran, CommandError> {
        IpcRequest::FriendAddFromInvite {
            invite: invite_string,
        }
    }
}

host_command! {
    state;
    /// Block anyone by public key (`IpcRequest::FriendBlock`): our profile
    /// moves to a new key.
    pub async fn block_user(
        public_key: String,
        state: State<'_, SharedState>,
    ) -> Result<Ran, CommandError> {
        IpcRequest::FriendBlock { public_key }
    }
}

host_command! {
    state;
    /// Unblock a user (`IpcRequest::FriendUnblock`); they are not re-added.
    pub async fn unblock_user(
        public_key: String,
        state: State<'_, SharedState>,
    ) -> Result<Ran, CommandError> {
        IpcRequest::FriendUnblock { public_key }
    }
}

host_command! {
    state;
    /// Everyone we blocked (`IpcRequest::FriendBlockedList`).
    pub async fn get_blocked_users(
        state: State<'_, SharedState>,
    ) -> Result<Done<Vec<BlockedUser>>, CommandError> {
        IpcRequest::FriendBlockedList
    }
}

host_command! {
    state;
    /// Cancel an open invite (`IpcRequest::FriendInviteCancel`).
    pub async fn cancel_invite(
        invite_id: String,
        state: State<'_, SharedState>,
    ) -> Result<Ran, CommandError> {
        IpcRequest::FriendInviteCancel { invite_id }
    }
}

host_command! {
    state;
    /// Our open invites (`IpcRequest::FriendInviteList`).
    pub async fn get_outgoing_invites(
        state: State<'_, SharedState>,
    ) -> Result<Done<Vec<OutgoingInvite>>, CommandError> {
        IpcRequest::FriendInviteList
    }
}
