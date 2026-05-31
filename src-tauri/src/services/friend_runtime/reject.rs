//! Phase 23.C — reject_request orchestration lifted from
//! `commands/friends.rs`. Read pending data, delete the pending row,
//! mark invite rejected (if from an invite), cache route blob, send
//! friend-reject envelope via Veilid.

use std::sync::Arc;

use crate::db::DbPool;
use crate::db_helpers::db_call;
use crate::services;
use crate::state::AppState;
use crate::state_helpers;

use super::read_pending_request_data;

pub async fn reject_request_inner(
    state: Arc<AppState>,
    pool: DbPool,
    public_key: String,
) -> Result<(), String> {
    let owner_key = state_helpers::current_owner_key(&state)?;

    let (_, _, pending_route_blob, _, invite_id) =
        read_pending_request_data(&pool, &owner_key, &public_key).await?;

    let pk = public_key.clone();
    let ok = owner_key.clone();
    db_call(&pool, move |conn| {
        conn.execute(
            "DELETE FROM pending_friend_requests WHERE owner_key = ?1 AND public_key = ?2",
            rusqlite::params![ok, pk],
        )?;
        Ok(())
    })
    .await?;

    if let Some(ref iid) = invite_id {
        crate::invite_helpers::mark_invite_rejected(&pool, &owner_key, iid);
    }

    if let Some(ref blob) = pending_route_blob {
        if !blob.is_empty() {
            let api = state_helpers::veilid_api(&state);
            if let Some(api) = api {
                let mut dht_mgr = state.dht_manager.write();
                if let Some(mgr) = dht_mgr.as_mut() {
                    mgr.manager.cache_route(&api, &public_key, blob.clone());
                }
            }
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
