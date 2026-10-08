//! Phase 23.C — reject_request orchestration lifted from
//! `commands/friends.rs`. Read pending data, delete the pending row,
//! mark invite rejected (if from an invite), cache route blob, send
//! friend-reject envelope via Veilid.

use std::sync::Arc;

use crate::db_helpers::db_call;
use crate::services;
use crate::state::AppState;
use crate::state_helpers;
use rekindle_db::Db;

use super::read_pending_request_data;

pub async fn reject_request_inner(
    state: Arc<AppState>,
    pool: Db,
    public_key: String,
) -> Result<(), String> {
    let owner_key = state_helpers::current_owner_key(&state)?;

    let rekindle_db::repo::pending_requests::Answer {
        route_blob: pending_route_blob,
        invite_id,
        ..
    } = read_pending_request_data(&pool, &owner_key, &public_key).await?;

    let pk = public_key.clone();
    let ok = owner_key.clone();
    db_call(&pool, move |conn| {
        rekindle_db::repo::pending_requests::delete(conn, &ok, &pk)
    })
    .await?;

    if let Some(ref iid) = invite_id {
        crate::invite_helpers::mark_invite_rejected(&pool, &owner_key, iid);
    }

    if let Some(ref blob) = pending_route_blob {
        if !blob.is_empty() {
            state_helpers::cache_peer_route(&state, &public_key, blob.clone());
        }
    }

    services::message_service::send_friend_reject(&state, &pool, &public_key)
        .await
        .unwrap_or_else(|e| {
            tracing::warn!(error = %e, "failed to send friend reject via Veilid (peer may be offline)");
        });

    tracing::info!(public_key = %public_key, "friend request rejected");
    Ok(())
}
