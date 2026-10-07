//! Phase 23.C — friend-handler Tauri-runtime orchestration lifted
//! from `commands/friends.rs`. Same pattern as `login_runtime.rs`:
//! the actual `#[tauri::command]` handler stays in `commands/` as a
//! ≤20-LoC delegation, and the wiring (DB writes + AppState mutation
//! + Veilid IO + Signal session establishment + audit-chain append)
//! lives here.
//!
//! Phase 14.r module-dir split applied — each function is its own
//! submodule (`add`, `invite`, `accept`, `remove`, `rotate`) so no
//! single file exceeds the ≤500-LoC behavior cap per Invariant 1.
//! Public surface unchanged: callers still
//! `crate::services::friend_runtime::accept_request_inner` etc.

use crate::db_helpers::db_call;
use rekindle_db::Db;

mod accept;
mod add;
mod block;
mod blocked;
mod cancel;
mod emit_presence;
mod from_invite;
mod generate_invite;
mod group_move;
mod groups;
mod invite;
mod list;
mod nickname;
mod outgoing_invites;
mod pending;
mod reject;
mod remove;
mod rotate;
mod session_reset;

pub use accept::accept_request_inner;
pub use add::add_friend_inner;
pub use block::block_user_inner;
pub use blocked::{get_blocked_users_inner, is_user_blocked, unblock_user_inner, BlockedUser};
pub use cancel::cancel_request_inner;
pub use emit_presence::emit_friends_presence_inner;
pub use from_invite::{add_friend_from_invite_inner, MAX_INVITE_AGE_SECS};
pub use generate_invite::{generate_invite_inner, GenerateInviteResult};
pub use group_move::move_friend_to_group_inner;
pub use groups::{create_friend_group_inner, rename_friend_group_inner};
pub use invite::setup_invite_contact;
pub use list::{list_friends_inner, FriendResponse};
pub use nickname::set_friend_nickname_inner;
pub use outgoing_invites::{cancel_invite_inner, get_outgoing_invites_inner};
pub use pending::{get_pending_requests_inner, PendingFriendRequest};
pub use reject::reject_request_inner;
pub use remove::remove_friend_inner;
pub use rotate::rotate_profile_key;
pub use session_reset::{
    accept_session_reset_inner, decline_session_reset_inner, reset_signal_session_inner,
};

/// What answering the pending request from `public_key` needs.
pub async fn read_pending_request_data(
    pool: &Db,
    owner_key: &str,
    public_key: &str,
) -> Result<rekindle_db::repo::pending_requests::Answer, String> {
    let ok = owner_key.to_string();
    let pk = public_key.to_string();
    db_call(pool, move |conn| {
        rekindle_db::repo::pending_requests::answer(conn, &ok, &pk)
    })
    .await
}

/// W11.3 — read the auto-volunteer Strand Relay preference from the
/// Tauri Store. Defaults to `false` (no auto-volunteer) on any error
/// or when the field is absent — explicit consent per
/// `feedback_vulnerable_users_no_creative_paths.md`.
pub fn auto_volunteer_relay_enabled(app: &tauri::AppHandle) -> bool {
    use tauri_plugin_store::StoreExt;
    let Ok(store) = app.store("preferences.json") else {
        return false;
    };
    let Some(val) = store.get("preferences") else {
        return false;
    };
    val.get("autoVolunteerRelayForNewFriends")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
}
