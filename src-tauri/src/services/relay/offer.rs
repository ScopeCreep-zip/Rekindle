//! Carol's side of Strand Relay (architecture §13.2 step 1-2):
//! allocate a dedicated private route on Veilid distinct from her
//! personal route, persist the (friend → route_id) mapping, and ship
//! the route blob to the friend over `app_message`.

use std::sync::Arc;

use rekindle_protocol::messaging::envelope::MessagePayload;

use rekindle_db::Db;
use rusqlite::OptionalExtension as _;

use crate::db_helpers::{db_call, db_call_or_default};
use crate::services::message_service;
use crate::state::AppState;
use crate::state_helpers;

/// What becomes of the relay route a new offer replaces (plan C7.9f).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Replaced {
    /// It is live: release it, or it leaks one route per re-volunteer.
    Release,
    /// Veilid reported it dead and already dropped it: releasing it again
    /// is an `InvalidArgument` Veilid logs at ERROR.
    Dead,
}

/// Volunteer to relay messages to `friend_public_key` (Carol → Bob).
///
/// 1. Allocate a fresh Veilid private route distinct from our personal one.
/// 2. Persist `(friend_public_key → route_id, blob)` so the inbound
///    `RelayEnvelope` dispatcher knows which friend is the intended target;
///    a route this replaces is released.
/// 3. Send a `RelayOffer` via `app_message` so Bob can publish the blob in
///    his pool.
pub async fn volunteer_relay(
    state: &Arc<AppState>,
    pool: &Db,
    friend_public_key: &str,
) -> Result<(), String> {
    offer_relay(state, pool, friend_public_key, Replaced::Release).await
}

/// Veilid reported these local routes dead (`RouteChange`): each friend
/// whose relay route is among them gets a fresh offer, so their pool does
/// not keep a route that reaches nobody (plan C7.9f). Our own routes are
/// not in the table, so this matches relay routes only.
pub async fn on_dead_relay_routes(state: &Arc<AppState>, pool: &Db, dead: &[veilid_core::RouteId]) {
    let owner_key = state_helpers::owner_key_or_default(state);
    if owner_key.is_empty() || dead.is_empty() {
        return;
    }
    let dead_ids: Vec<String> = dead.iter().map(ToString::to_string).collect();
    let friends: Vec<String> = db_call_or_default(pool, move |conn| {
        let mut stmt = conn.prepare(
            "SELECT friend_public_key, relay_route_id FROM strand_relay_volunteered \
             WHERE owner_key = ?1",
        )?;
        let rows = stmt.query_map(rusqlite::params![owner_key], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        let rows = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows
            .into_iter()
            .filter(|(_, route_id)| dead_ids.contains(route_id))
            .map(|(friend, _)| friend)
            .collect())
    })
    .await;
    for friend in friends {
        match offer_relay(state, pool, &friend, Replaced::Dead).await {
            Ok(()) => tracing::info!(friend = %friend, "relay route died; re-volunteered"),
            Err(e) => {
                tracing::warn!(friend = %friend, error = %e, "relay route died; re-volunteer failed");
            }
        }
    }
}

async fn offer_relay(
    state: &Arc<AppState>,
    pool: &Db,
    friend_public_key: &str,
    replaced: Replaced,
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
    // Read the route this replaces and upsert in ONE db_call, so a
    // concurrent offer cannot slip between and leak the id read.
    let old_route_id: Option<String> = db_call(pool, move |conn| {
        let old: Option<String> = conn
            .query_row(
                "SELECT relay_route_id FROM strand_relay_volunteered \
                 WHERE owner_key = ?1 AND friend_public_key = ?2",
                rusqlite::params![&pseudonym, &friend],
                |row| row.get(0),
            )
            .optional()?;
        conn.execute(
            "INSERT INTO strand_relay_volunteered (owner_key, friend_public_key, relay_route_id, relay_route_blob, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(owner_key, friend_public_key) DO UPDATE SET
                 relay_route_id = excluded.relay_route_id,
                 relay_route_blob = excluded.relay_route_blob,
                 created_at = excluded.created_at",
            rusqlite::params![pseudonym, friend, route_id_for_db, blob_for_db, now],
        )?;
        Ok(old)
    })
    .await?;
    if replaced == Replaced::Release {
        if let Some(old) = old_route_id.filter(|old| *old != route_id) {
            release_relay_route(&api, &old);
        }
    }

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
    pool: &Db,
    friend_public_key: &str,
) -> Result<(), String> {
    let owner_key = state_helpers::owner_key_or_default(state);
    if owner_key.is_empty() {
        return Err("no identity".into());
    }
    let pseudonym_for_payload = owner_key.clone();
    let pseudonym = owner_key;
    let friend = friend_public_key.to_string();
    // Read the route id and delete in ONE db_call: a SELECT-then-DELETE
    // in two calls can race a concurrent revoke and leak the very id it
    // was about to read.
    let route_id: Option<String> = db_call(pool, move |conn| {
        let existing: Option<String> = conn
            .query_row(
                "SELECT relay_route_id FROM strand_relay_volunteered \
                 WHERE owner_key = ?1 AND friend_public_key = ?2",
                rusqlite::params![&pseudonym, &friend],
                |row| row.get(0),
            )
            .optional()?;
        conn.execute(
            "DELETE FROM strand_relay_volunteered WHERE owner_key = ?1 AND friend_public_key = ?2",
            rusqlite::params![pseudonym, friend],
        )?;
        Ok(existing)
    })
    .await?;

    // The route outlives the row unless we say so. `release_private_route`
    // is local with no network round-trip; an unknown or already-released
    // id returns InvalidArgument, which is not worth failing a revoke
    // over — but silently leaking one route per volunteer/revoke cycle is.
    if let Some(id) = route_id {
        if let Some(api) = state_helpers::veilid_api(state) {
            release_relay_route(&api, &id);
        } else {
            tracing::debug!(route_id = %id, "relay route not released — api unavailable");
        }
    }

    let payload = MessagePayload::RelayWithdraw {
        relay_pseudonym: pseudonym_for_payload,
    };
    let _ = message_service::send_to_peer(state, pool, friend_public_key, &payload).await;
    Ok(())
}

/// Release a live relay route by its stored id.
fn release_relay_route(api: &veilid_core::VeilidAPI, id: &str) {
    match id.parse::<veilid_core::RouteId>() {
        Ok(parsed) => {
            if let Err(e) = api.release_private_route(parsed) {
                tracing::debug!(route_id = %id, error = %e, "relay route release");
            }
        }
        Err(e) => tracing::debug!(route_id = %id, error = %e, "relay route id unreadable"),
    }
}

/// List friends we've volunteered to relay for. Used by the buddy-list
/// context menu to swap "Volunteer to relay" for "Stop relaying" without
/// a server round-trip.
pub async fn list_volunteered_for(state: &Arc<AppState>, pool: &Db) -> Vec<String> {
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
