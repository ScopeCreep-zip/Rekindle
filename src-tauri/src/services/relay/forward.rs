//! Carol's relay forwarding (architecture §13.3 step 3): on receipt of
//! a `RelayEnvelope` whose `target_pubkey` matches a friend we've
//! actively volunteered to relay for, forward `inner_payload` verbatim
//! to that friend's current Veilid route. Drop otherwise — Carol must
//! not become a free relay for arbitrary friends, only those who have
//! her offer in their pool.

use std::sync::Arc;

use crate::db::DbPool;
use crate::db_helpers::db_call_or_default;
use crate::state::AppState;
use crate::state_helpers;

pub async fn handle_relay_envelope(
    state: &Arc<AppState>,
    pool: &DbPool,
    target_pubkey: &str,
    inner_payload: &[u8],
) -> Result<(), String> {
    if !state_helpers::is_friend(state, target_pubkey) {
        return Err("relay target is not a known friend".into());
    }

    // Architecture §13.2 step 1: forwarding requires that we *actively*
    // volunteered for this friend. Reject otherwise — without this we'd
    // forward for any friend referenced, opening Carol up to abuse.
    let owner_key = state_helpers::owner_key_or_default(state);
    if owner_key.is_empty() {
        return Err("no identity".into());
    }
    let owner = owner_key;
    let target = target_pubkey.to_string();
    let volunteered: bool = db_call_or_default(pool, move |conn| {
        Ok(conn
            .query_row(
                "SELECT 1 FROM strand_relay_volunteered
                 WHERE owner_key = ?1 AND friend_public_key = ?2 LIMIT 1",
                rusqlite::params![owner, target],
                |_| Ok(()),
            )
            .is_ok())
    })
    .await;
    if !volunteered {
        return Err("not volunteered to relay for this friend".into());
    }

    let route_id_and_rc = state_helpers::try_import_peer_route(state, target_pubkey);
    let Some((route_id, routing_context)) = route_id_and_rc else {
        return Err("no cached route for relay target".into());
    };

    routing_context
        .app_message(veilid_core::Target::RouteId(route_id), inner_payload.to_vec())
        .await
        .map_err(|e| format!("relay app_message failed: {e}"))?;
    Ok(())
}
