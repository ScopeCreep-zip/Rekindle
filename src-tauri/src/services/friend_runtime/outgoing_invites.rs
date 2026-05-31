//! Phase 23.C — outgoing-invite list/cancel handlers lifted from
//! `commands/friends.rs`. `setup_invite_contact` + `generate_invite`
//! orchestrators live in `invite.rs` and `generate_invite.rs`; this
//! module covers the two read/delete operations on the tracked
//! `outgoing_invites` SQLite table.

use crate::db::DbPool;
use crate::state::SharedState;
use crate::state_helpers;

pub async fn cancel_invite_inner(
    state: &SharedState,
    pool: &DbPool,
    invite_id: String,
) -> Result<(), String> {
    let owner_key = state_helpers::current_owner_key(state)?;
    crate::invite_helpers::cancel_outgoing_invite(pool, &owner_key, &invite_id).await?;
    tracing::info!(%invite_id, "invite cancelled");
    Ok(())
}

pub async fn get_outgoing_invites_inner(
    state: &SharedState,
    pool: &DbPool,
) -> Result<Vec<crate::invite_helpers::OutgoingInvite>, String> {
    let owner_key = state_helpers::current_owner_key(state)?;
    crate::invite_helpers::get_pending_invites(pool, &owner_key).await
}
