//! Carol's side of Strand Relay (architecture §13.2 step 1-2):
//! allocate a dedicated private route on Veilid distinct from her
//! personal route, persist the (friend → route_id) mapping, and ship
//! the route blob to the friend over `app_message`.

use std::sync::Arc;

use rekindle_protocol::messaging::envelope::MessagePayload;

use crate::db::DbPool;
use crate::db_helpers::{db_call, db_call_or_default};
use crate::services::message_service;
use crate::state::AppState;
use crate::state_helpers;

/// Volunteer to relay messages to `friend_public_key` (Carol → Bob).
///
/// 1. Allocate a fresh Veilid private route distinct from our personal one.
/// 2. Persist `(friend_public_key → route_id, blob)` so the inbound
///    `RelayEnvelope` dispatcher knows which friend is the intended target.
/// 3. Send a `RelayOffer` via `app_message` so Bob can publish the blob in
///    his pool.
pub async fn volunteer_relay(
    state: &Arc<AppState>,
    pool: &DbPool,
    friend_public_key: &str,
) -> Result<(), String> {
    let api =
        state_helpers::veilid_api(state).ok_or_else(|| "veilid api unavailable".to_string())?;
    let owner_key = state_helpers::owner_key_or_default(state);
    if owner_key.is_empty() {
        return Err("no identity".into());
    }
    let route = api
        .new_private_route()
        .await
        .map_err(|e| format!("new_private_route: {e}"))?;
    let route_id = route.route_id.to_string();
    let route_blob = route.blob;

    let pseudonym = owner_key.clone();
    let friend = friend_public_key.to_string();
    let route_id_for_db = route_id.clone();
    let blob_for_db = route_blob.clone();
    let now = crate::db::timestamp_now();
    db_call(pool, move |conn| {
        conn.execute(
            "INSERT INTO strand_relay_volunteered (owner_key, friend_public_key, relay_route_id, relay_route_blob, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(owner_key, friend_public_key) DO UPDATE SET
                 relay_route_id = excluded.relay_route_id,
                 relay_route_blob = excluded.relay_route_blob,
                 created_at = excluded.created_at",
            rusqlite::params![pseudonym, friend, route_id_for_db, blob_for_db, now],
        )?;
        Ok(())
    })
    .await?;

    let payload = MessagePayload::RelayOffer {
        relay_route_blob: route_blob,
        relay_pseudonym: owner_key,
    };
    // Architecture §13.2 step 2 — `app_call` so we get a persisted-ack
    // reply. If the friend's pool persist failed (disk full, schema
    // mismatch) we surface the error rather than silently leaving the
    // friend without our route.
    let reply = message_service::send_to_peer_call(state, friend_public_key, &payload).await?;
    match reply {
        MessagePayload::RelayOfferAck { ok: true, .. } => Ok(()),
        MessagePayload::RelayOfferAck { ok: false, reason } => Err(if reason.is_empty() {
            "friend rejected RelayOffer".to_string()
        } else {
            format!("friend rejected RelayOffer: {reason}")
        }),
        other => Err(format!("unexpected RelayOffer reply: {other:?}")),
    }
}

/// Revoke a previously volunteered relay (Carol withdraws).
pub async fn revoke_relay(
    state: &Arc<AppState>,
    pool: &DbPool,
    friend_public_key: &str,
) -> Result<(), String> {
    let owner_key = state_helpers::owner_key_or_default(state);
    if owner_key.is_empty() {
        return Err("no identity".into());
    }
    let pseudonym_for_payload = owner_key.clone();
    let pseudonym = owner_key;
    let friend = friend_public_key.to_string();
    db_call(pool, move |conn| {
        conn.execute(
            "DELETE FROM strand_relay_volunteered WHERE owner_key = ?1 AND friend_public_key = ?2",
            rusqlite::params![pseudonym, friend],
        )?;
        Ok(())
    })
    .await?;

    let payload = MessagePayload::RelayWithdraw {
        relay_pseudonym: pseudonym_for_payload,
    };
    let _ = message_service::send_to_peer_raw(state, pool, friend_public_key, &payload).await;
    Ok(())
}

/// List friends we've volunteered to relay for. Used by the buddy-list
/// context menu to swap "Volunteer to relay" for "Stop relaying" without
/// a server round-trip.
pub async fn list_volunteered_for(state: &Arc<AppState>, pool: &DbPool) -> Vec<String> {
    let owner_key = state_helpers::owner_key_or_default(state);
    if owner_key.is_empty() {
        return Vec::new();
    }
    db_call_or_default(pool, move |conn| {
        let mut stmt = conn.prepare(
            "SELECT friend_public_key FROM strand_relay_volunteered WHERE owner_key = ?1",
        )?;
        let rows = stmt.query_map(rusqlite::params![owner_key], |row| row.get::<_, String>(0))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
    })
    .await
}
