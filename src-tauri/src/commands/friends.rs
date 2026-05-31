use tauri::State;

use crate::db::DbPool;
use crate::services::friend_runtime::{
    accept_request_inner, accept_session_reset_inner, add_friend_from_invite_inner,
    add_friend_inner, block_user_inner, cancel_invite_inner, cancel_request_inner,
    create_friend_group_inner, decline_session_reset_inner, emit_friends_presence_inner,
    generate_invite_inner, get_blocked_users_inner, get_outgoing_invites_inner,
    get_pending_requests_inner, list_friends_inner, move_friend_to_group_inner,
    reject_request_inner, remove_friend_inner, rename_friend_group_inner,
    reset_signal_session_inner, unblock_user_inner, BlockedUser, FriendResponse,
    GenerateInviteResult, PendingFriendRequest,
};

pub use crate::services::friend_runtime::is_user_blocked;
use crate::state::SharedState;

/// Get all persisted pending friend requests for the current identity.
#[tauri::command]
pub async fn get_pending_requests(
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<Vec<PendingFriendRequest>, String> {
    get_pending_requests_inner(state.inner(), pool.inner()).await
}

/// Add a friend by their public key.
///
/// Phase 8 — `idempotency_key` (UUID v7) dedupes click-spam. Without
/// it, double-tapping "Add" issues two friend requests (two DHT inbox
/// writes against the same target), spamming the recipient and
/// creating ambiguous request state.
#[tauri::command]
pub async fn add_friend(
    public_key: String,
    display_name: String,
    message: String,
    idempotency_key: uuid::Uuid,
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    // Phase 5 — gate writes on lifecycle.
    let _g = rekindle_lifecycle::TransportGuard::write(&state.lifecycle)
        .map_err(|e| e.to_string())?;
    let state_clone = state.inner().clone();
    let pool_clone = pool.inner().clone();
    state
        .idempotency
        .wrap(idempotency_key, || async move {
            add_friend_inner(state_clone, pool_clone, app, public_key, display_name, message).await
        })
        .await
}

/// Remove a friend.
#[tauri::command]
pub async fn remove_friend(
    public_key: String,
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    remove_friend_inner(state.inner().clone(), pool.inner().clone(), app, public_key).await
}

/// Accept a pending friend request.
#[tauri::command]
pub async fn accept_request(
    public_key: String,
    display_name: String,
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    accept_request_inner(
        state.inner().clone(),
        pool.inner().clone(),
        app,
        public_key,
        display_name,
    )
    .await
}

/// Get the full friends list.
#[tauri::command]
pub async fn get_friends(state: State<'_, SharedState>) -> Result<Vec<FriendResponse>, String> {
    let _g = rekindle_lifecycle::TransportGuard::read(&state.lifecycle)
        .map_err(|e| e.to_string())?;
    Ok(list_friends_inner(state.inner()))
}

/// Reject a pending friend request.
/// Sends a rejection message to the peer and removes the request from the database.
#[tauri::command]
pub async fn reject_request(
    public_key: String,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    reject_request_inner(state.inner().clone(), pool.inner().clone(), public_key).await
}

/// Cancel an outbound pending friend request.
///
/// Only works for friends in `PendingOut` state — removes them from the
/// friends table and in-memory state.
#[tauri::command]
pub async fn cancel_request(
    public_key: String,
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    cancel_request_inner(state.inner().clone(), pool.inner().clone(), app, public_key).await
}

/// B6 — explicit user-driven Signal session reset for a peer.
///
/// The hardened safety stance forbids auto-rehandshake on decrypt
/// failure (an attacker who corrupts session state could force an
/// auto-recovery that substitutes their own keys). Instead, the user
/// initiates the reset explicitly from the friend's context menu after
/// verifying their safety number out-of-band.
///
/// Effect: deletes the local Signal session record for the peer. The
/// next outbound encrypted message hits send_envelope_to_peer's "no
/// session with {peer}" error (B8) which prompts the user to verify
/// the peer's safety number before resuming. A fresh session is
/// established at the next FriendAccept-style handshake.
///
/// Idempotent — calling on a peer with no active session is a no-op.
///
/// P3.3 update — also sends a SessionResetRequest to the peer carrying
/// our fresh PreKeyBundle. The peer's UI surfaces a confirmation modal
/// (NotificationEvent::SessionResetRequested) before any session state
/// changes on their side; if they accept, they install a fresh session
/// and reply with SessionResetAccept which our message_service handles
/// to install our matching responder-side session. If they decline,
/// they send SessionResetDecline and our local session stays deleted
/// but no fresh session is established (caller knows the peer doesn't
/// want to renew).
#[tauri::command]
pub async fn reset_signal_session(
    peer_public_key: String,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    reset_signal_session_inner(state.inner().clone(), pool.inner().clone(), peer_public_key).await
}

/// P3.3 — accept a SessionResetRequest from a peer. Consumes the
/// stashed bundle from `state.pending_session_resets`, calls
/// establish_session(peer, peer_bundle), and sends SessionResetAccept
/// back with the X3DH metadata so the peer can complete the renewal on
/// their side via respond_to_session.
///
/// MUST only be invoked after the user has verified the peer's safety
/// number out-of-band. The frontend's session-reset modal is responsible
/// for showing the safety_number from the NotificationEvent and
/// requiring explicit user consent before invoking this command.
#[tauri::command]
pub async fn accept_session_reset(
    peer_public_key: String,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    accept_session_reset_inner(state.inner().clone(), pool.inner().clone(), peer_public_key).await
}

/// P3.3 — decline a SessionResetRequest. Clears the stashed bundle and
/// sends SessionResetDecline so the peer knows we said no (their UI can
/// surface the rejection so the user understands the renewal didn't
/// complete).
#[tauri::command]
pub async fn decline_session_reset(
    peer_public_key: String,
    reason: Option<String>,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    decline_session_reset_inner(
        state.inner().clone(),
        pool.inner().clone(),
        peer_public_key,
        reason,
    )
    .await
}

/// Create a new friend group.
#[tauri::command]
pub async fn create_friend_group(
    name: String,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<i64, String> {
    create_friend_group_inner(state.inner(), pool.inner(), name).await
}

/// Rename a friend group.
#[tauri::command]
pub async fn rename_friend_group(
    group_id: i64,
    name: String,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    rename_friend_group_inner(pool.inner(), group_id, name).await
}

/// Move a friend into a group (or remove from group with `group_id` = null).
#[tauri::command]
pub async fn move_friend_to_group(
    public_key: String,
    group_id: Option<i64>,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    move_friend_to_group_inner(state.inner().clone(), pool.inner().clone(), public_key, group_id)
        .await
}

/// Generate an invite link containing everything needed for a peer to add us.
#[tauri::command]
pub async fn generate_invite(
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<GenerateInviteResult, String> {
    generate_invite_inner(state.inner().clone(), pool.inner().clone()).await
}

/// Add a friend from a `rekindle://` invite string.
#[tauri::command]
pub async fn add_friend_from_invite(
    invite_string: String,
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    add_friend_from_invite_inner(state.inner().clone(), pool.inner().clone(), app, invite_string)
        .await
}

/// Block a user — works for any public key (friend, pending, invite, or raw key).
///
/// Removes them from friends/pending requests, adds to blocked list, cleans up
/// Signal session, pending messages, DHT state, and rotates our profile key.
#[tauri::command]
pub async fn block_user(
    public_key: String,
    display_name: Option<String>,
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    block_user_inner(
        state.inner().clone(),
        pool.inner().clone(),
        app,
        public_key,
        display_name,
    )
    .await
}

/// Unblock a user — removes them from the blocked list.
///
/// Does NOT re-add them as a friend. The user must manually re-add if desired.
#[tauri::command]
pub async fn unblock_user(
    public_key: String,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    unblock_user_inner(state.inner(), pool.inner(), public_key).await
}

/// Get all blocked users for the current identity.
#[tauri::command]
pub async fn get_blocked_users(
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<Vec<BlockedUser>, String> {
    get_blocked_users_inner(state.inner(), pool.inner()).await
}

/// Re-emit presence events for all non-offline friends.
///
/// Called by the frontend after hydration completes so that event listeners
/// (registered before hydration) receive the current friend presence state.
/// Waits for Veilid network readiness (up to 15s) before syncing from DHT
/// so that `state.friends` has fresh data rather than stale Offline defaults.
#[tauri::command]
pub async fn emit_friends_presence(
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
) -> Result<(), String> {
    emit_friends_presence_inner(state.inner().clone(), app).await
}

/// Cancel a pending outgoing invite by its `invite_id`.
#[tauri::command]
pub async fn cancel_invite(
    invite_id: String,
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<(), String> {
    cancel_invite_inner(state.inner(), pool.inner(), invite_id).await
}

/// Get all active (pending/responded) outgoing invites.
#[tauri::command]
pub async fn get_outgoing_invites(
    state: State<'_, SharedState>,
    pool: State<'_, DbPool>,
) -> Result<Vec<crate::invite_helpers::OutgoingInvite>, String> {
    get_outgoing_invites_inner(state.inner(), pool.inner()).await
}
