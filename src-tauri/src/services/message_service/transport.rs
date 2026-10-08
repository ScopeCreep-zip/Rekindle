//! Phase 23.D — outgoing-envelope transport layer lifted from
//! `message_service/mod.rs`. Handles cached-route lookup, inline DHT
//! route refresh, strand-relay fallback (architecture §13.3),
//! status-probe fan-out (architecture §13.5), and final
//! pending-message queue. Pure Veilid orchestration — protocol-level
//! envelope construction stays in `outgoing.rs` callers and `rekindle-protocol`.

use std::sync::Arc;

use rekindle_codec::message::envelope::{MessageEnvelope, MessagePayload};
use rekindle_protocol::messaging::sender::send_envelope;

use crate::state::AppState;
use crate::state_helpers;
use rekindle_db::Db;

use super::outgoing::queue_pending_message;

/// Fetch a fresh route blob inline from the peer's profile DHT record
/// (subkey 6) and cache it. Returns the blob on success.
pub(crate) async fn try_fetch_route_from_dht(
    state: &Arc<AppState>,
    peer_id: &str,
) -> Option<Vec<u8>> {
    let dht_key = state_helpers::friend_dht_key(state, peer_id)?;
    let pool = state_helpers::record_pool(state).ok()?;
    let route_blob = rekindle_protocol::dht::profile::read_profile_subkey(
        &pool,
        &dht_key,
        rekindle_protocol::dht::profile::SUBKEY_ROUTE_BLOB,
        true,
    )
    .await
    .ok()??;
    if route_blob.is_empty() {
        return None;
    }
    state_helpers::cache_peer_route(state, peer_id, route_blob.clone());
    tracing::debug!(peer = %peer_id, "fetched fresh route blob from DHT inline");
    Some(route_blob)
}

/// Try to fetch a fresh route from DHT and send the envelope immediately.
///
/// Used as an inline recovery path when no cached route exists or after a send
/// failure, avoiding the 30-second sync loop wait. Returns `true` if the send
/// succeeded, `false` if no route could be obtained or the send failed.
async fn try_inline_route_refresh_and_send(
    state: &Arc<AppState>,
    to: &str,
    envelope: &MessageEnvelope,
) -> bool {
    let Some(fresh_blob) = try_fetch_route_from_dht(state, to).await else {
        return false;
    };

    let retry = state_helpers::safe_routing_context(state).and_then(|rc| {
        state_helpers::import_route_blob(state, &fresh_blob)
            .ok()
            .map(|rid| (rid, rc))
    });

    if let Some((rid, rc)) = retry {
        let sent = send_envelope(&rc, rid.clone(), envelope).await;
        if state_helpers::note_send_result(state, &rid, sent).is_ok() {
            return true;
        }
    }

    false
}

/// Spawn a background `StatusRequest` fan-out (architecture §13.5) so
/// any relay friend that holds a fresh snapshot of `to` can update our
/// local cache. Best-effort — does not block the caller.
fn probe_relay_friends_for_status(state: &Arc<AppState>, pool: &Db, to: &str) {
    let state_clone = state.clone();
    let pool_clone = pool.clone();
    let target = to.to_string();
    crate::state_helpers::login_scope_or_closed(state).spawn_or_drop(
        "relay status probe",
        async move {
            crate::services::relay::presence::probe_friends_for_status(
                &state_clone,
                &pool_clone,
                &target,
            )
            .await;
        },
    );
}

/// Strand Relay last-resort fallback (architecture §13.3): when both the
/// cached route and inline-DHT-refresh fail, look up the recipient's
/// published relay pool (profile DHT subkey 8) and forward the
/// already-built envelope through a random non-dummy relay friend.
/// The friend profile record itself is kept warm by
/// `sync_service::sync_friend_dht_subkeys`, which reads subkeys 2/4/5/6
/// every 30 seconds — Veilid's per-record TTL covers subkey 8 by
/// association, so we don't need a dedicated keepalive.
async fn try_relay_fallback_send(
    state: &Arc<AppState>,
    to: &str,
    envelope: &MessageEnvelope,
) -> bool {
    let Some(dht_key) = state_helpers::friend_dht_key(state, to) else {
        return false;
    };
    let Ok(pool) = state_helpers::record_pool(state) else {
        return false;
    };
    let Ok(Some(pool_body)) = rekindle_protocol::dht::profile::read_profile_subkey(
        &pool,
        &dht_key,
        rekindle_protocol::dht::profile::SUBKEY_RELAY_POOL,
        true,
    )
    .await
    else {
        return false;
    };
    let Ok(envelope_bytes) = serde_json::to_vec(envelope) else {
        return false;
    };
    crate::services::relay::send::send_via_relay(state, to, &pool_body, &envelope_bytes)
        .await
        .is_ok()
}

/// Seal `payload` as its [`rekindle_codec::message::envelope::Sealing`]
/// requires, sign it to `to`, and send via Veilid.
///
/// If no route exists for the peer, the message is queued for retry by `sync_service`.
/// Ephemeral payloads (typing indicators) are never queued — a stale typing indicator
/// delivered minutes later is worse than no indicator.
pub(super) async fn send_envelope_to_peer(
    state: &Arc<AppState>,
    pool: &Db,
    to: &str,
    payload: &MessagePayload,
) -> Result<(), String> {
    let is_ephemeral = matches!(payload, MessagePayload::TypingIndicator { .. });
    let envelope = super::seal::build_signed_envelope(state, to, payload).await?;

    // Look up the peer's cached route blob and import the RouteId via cache
    let route_id_and_rc = state_helpers::try_import_peer_route(state, to);

    let Some((route_id, routing_context)) = route_id_and_rc else {
        if is_ephemeral {
            tracing::debug!(to = %to, "no cached route for peer — dropping ephemeral message");
            return Ok(());
        }
        // Inline DHT route re-fetch before queuing — avoids 30s wait for sync loop
        if try_inline_route_refresh_and_send(state, to, &envelope).await {
            tracing::info!(to = %to, "message sent via veilid (after inline route refresh)");
            return Ok(());
        }
        // Strand Relay fallback (architecture §13.3): try a mutual friend's
        // published relay pool before giving up to the queue.
        if try_relay_fallback_send(state, to, &envelope).await {
            tracing::info!(to = %to, "message sent via strand relay");
            return Ok(());
        }
        // Architecture §13.5: ask our friends if any of them hold a
        // cached status (and a fresh route blob) for the target. Any
        // late-arriving StatusResponse will rehydrate the route cache
        // for the next send.
        probe_relay_friends_for_status(state, pool, to);
        tracing::debug!(to = %to, "no cached route for peer — queuing message for retry");
        let envelope_json =
            serde_json::to_string(&envelope).map_err(|e| format!("serialize envelope: {e}"))?;
        queue_pending_message(state, pool, to, &envelope_json).await?;
        return Ok(());
    };

    let sent = send_envelope(&routing_context, route_id.clone(), &envelope).await;
    if let Err(e) = state_helpers::note_send_result(state, &route_id, sent) {
        // Forget the stale cached blob so the next retry fetches fresh from DHT
        state_helpers::invalidate_cached_peer_route(state, to);
        if is_ephemeral {
            tracing::debug!(to = %to, error = %e, "send failed — dropping ephemeral message");
            return Ok(());
        }
        // Inline DHT route re-fetch before queuing — avoids 30s wait for sync loop
        if try_inline_route_refresh_and_send(state, to, &envelope).await {
            tracing::info!(to = %to, "message sent via veilid (after send failure + inline route refresh)");
            return Ok(());
        }
        if try_relay_fallback_send(state, to, &envelope).await {
            tracing::info!(to = %to, "message sent via strand relay (after send failure)");
            return Ok(());
        }
        tracing::warn!(to = %to, error = %e, "send failed — queuing for retry");
        let envelope_json =
            serde_json::to_string(&envelope).map_err(|e| format!("serialize envelope: {e}"))?;
        queue_pending_message(state, pool, to, &envelope_json).await?;
        return Ok(());
    }

    tracing::info!(to = %to, "message sent via veilid");
    Ok(())
}
