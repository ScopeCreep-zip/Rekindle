//! Carol's relay forwarding (architecture §13.3 step 3): on receipt of
//! a `RelayEnvelope` (0x0003) whose `target_pubkey` matches a friend
//! we've actively volunteered to relay for, forward `inner_payload` — a
//! whole `'D'` datagram from the sender to the target, sealed under their
//! session — verbatim to that friend's current Veilid route. Drop
//! otherwise — Carol must not become a free relay for arbitrary friends,
//! only those who have her offer in their pool, and she forwards only 1:1
//! datagrams.

use std::sync::Arc;

use crate::db_helpers::db_call;
use crate::state::AppState;
use crate::state_helpers;
use rekindle_db::Db;
use rusqlite::OptionalExtension;

pub async fn handle_relay_envelope(
    state: &Arc<AppState>,
    pool: &Db,
    target_pubkey: &str,
    inner_payload: &[u8],
) -> Result<(), String> {
    if !state_helpers::is_friend(state, target_pubkey) {
        return Err("relay target is not a known friend".into());
    }
    if rekindle_transport::operations::dm::unwire(inner_payload).is_none() {
        return Err("relay payload is not a 1:1 datagram".into());
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
    let volunteered: bool = db_call(pool, move |conn| {
        Ok(conn
            .query_row(
                "SELECT 1 FROM strand_relay_volunteered
                 WHERE owner_key = ?1 AND friend_public_key = ?2 LIMIT 1",
                rusqlite::params![owner, target],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    })
    .await?;
    if !volunteered {
        return Err("not volunteered to relay for this friend".into());
    }

    // The target's route from the one peer-route cache (plan E2.1); the
    // inner bytes are forwarded as they came.
    let node = state
        .transport
        .read()
        .clone()
        .ok_or("transport not started")?;
    let blob = rekindle_transport::operations::dm::route::cached(&node, target_pubkey)
        .ok_or("no cached route for relay target")?;
    let target = node
        .routes()
        .import(&blob)
        .map_err(|e| format!("relay target route: {e}"))?;
    node.sender()
        .send_direct(&target, inner_payload.to_vec())
        .await
        .map_err(|e| format!("relay app_message failed: {e}"))
}
