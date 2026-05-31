//! Phase 23.D — outgoing-envelope transport layer lifted from
//! `message_service/mod.rs`. Handles cached-route lookup, inline DHT
//! route refresh, strand-relay fallback (architecture §13.3),
//! status-probe fan-out (architecture §13.5), and final
//! pending-message queue. Pure Veilid orchestration — protocol-level
//! envelope construction stays in `outgoing.rs` callers and `rekindle-protocol`.

use std::sync::Arc;

use rand::RngCore as _;
use rekindle_protocol::messaging::envelope::{MessageEnvelope, MessagePayload};
use rekindle_protocol::messaging::sender::{build_envelope_from_secret, send_envelope};

use crate::db::DbPool;
use crate::state::AppState;
use crate::state_helpers;

use super::outgoing::queue_pending_message;

/// Fetch a fresh route blob inline from the peer's profile DHT record
/// (subkey 6) and cache it. Returns the blob on success.
pub(crate) async fn try_fetch_route_from_dht(
    state: &Arc<AppState>,
    peer_id: &str,
) -> Option<Vec<u8>> {
    let dht_key_str = state_helpers::friend_dht_key(state, peer_id)?;

    let record_key: veilid_core::RecordKey = dht_key_str.parse().ok()?;

    let (api, routing_context) = state_helpers::safe_api_and_routing_context(state)?;

    // Open (no-op if already open) and force-refresh subkey 6 (route blob)
    let _ = routing_context
        .open_dht_record(record_key.clone(), None)
        .await;
    let value_data = routing_context
        .get_dht_value(record_key, 6, true)
        .await
        .ok()??;
    let route_blob = value_data.data().to_vec();
    if route_blob.is_empty() {
        return None;
    }

    // Cache the fresh route blob
    {
        let mut dht_mgr = state.dht_manager.write();
        if let Some(mgr) = dht_mgr.as_mut() {
            mgr.manager.cache_route(&api, peer_id, route_blob.clone());
        }
    }

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
        if send_envelope(&rc, rid, envelope).await.is_ok() {
            return true;
        }
    }

    false
}

/// Spawn a background `StatusRequest` fan-out (architecture §13.5) so
/// any relay friend that holds a fresh snapshot of `to` can update our
/// local cache. Best-effort — does not block the caller.
fn probe_relay_friends_for_status(state: &Arc<AppState>, pool: &DbPool, to: &str) {
    let state_clone = state.clone();
    let pool_clone = pool.clone();
    let target = to.to_string();
    tokio::spawn(async move {
        crate::services::relay::presence::probe_friends_for_status(
            &state_clone,
            &pool_clone,
            &target,
        )
        .await;
    });
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
    let Some(dht_key_str) = state_helpers::friend_dht_key(state, to) else {
        return false;
    };
    let Ok(record_key) = dht_key_str.parse::<veilid_core::RecordKey>() else {
        return false;
    };
    let Some(routing_context) = state_helpers::safe_routing_context(state) else {
        return false;
    };
    let _ = routing_context
        .open_dht_record(record_key.clone(), None)
        .await;
    let pool_body = match routing_context
        .get_dht_value(
            record_key,
            rekindle_protocol::dht::profile::SUBKEY_RELAY_POOL,
            true,
        )
        .await
    {
        Ok(Some(v)) => v.data().to_vec(),
        _ => return false,
    };
    let Ok(envelope_bytes) = serde_json::to_vec(envelope) else {
        return false;
    };
    crate::services::relay::send::send_via_relay(state, to, &pool_body, &envelope_bytes)
        .await
        .is_ok()
}

/// Build a `MessageEnvelope`, optionally encrypt with Signal, and send via Veilid.
///
/// If no route exists for the peer, the message is queued for retry by `sync_service`.
/// Ephemeral payloads (typing indicators) are never queued — a stale typing indicator
/// delivered minutes later is worse than no indicator.
pub(super) async fn send_envelope_to_peer(
    state: &Arc<AppState>,
    pool: &DbPool,
    to: &str,
    payload: &MessagePayload,
    encrypt: bool,
) -> Result<(), String> {
    let is_ephemeral = matches!(payload, MessagePayload::TypingIndicator { .. });
    // Serialize the payload
    let payload_bytes =
        serde_json::to_vec(payload).map_err(|e| format!("serialize payload: {e}"))?;

    // B8/P3.3 — when encrypt is requested, refuse to fall back to plaintext.
    // The previous `_ => payload_bytes` arm silently sent the body in clear
    // when has_session returned false or errored, which an active attacker
    // who can corrupt your local Signal session state could trigger to
    // intercept the plaintext.
    //
    // Vulnerable-user safety stance: fail-closed. The caller's Result<(),
    // String> propagates to the Tauri IPC layer; the frontend toast shows
    // the specific reason and points the user at the explicit
    // "Re-establish secure session" path. No silent downgrade.
    let final_payload = if encrypt {
        // Phase 6 — clone the Arc out so we can `.await` on the now-async
        // encrypt without holding the parking_lot read-guard.
        let handle = state
            .signal_manager
            .read()
            .as_ref()
            .map(std::sync::Arc::clone)
            .ok_or_else(|| {
                format!(
                    "No Signal manager — cannot send encrypted message to {to}. \
                     Sign in to initialize Signal sessions."
                )
            })?;
        match handle.manager.has_session(to) {
            Ok(true) => handle
                .manager
                .encrypt(to, &payload_bytes)
                .await
                .map_err(|e| format!("Signal encrypt failed for {to}: {e}"))?,
            Ok(false) => {
                return Err(format!(
                    "No secure session with {to}. They haven't completed a Signal handshake yet — \
                     they need to accept your friend request before you can send encrypted messages. \
                     Verify their safety number out-of-band before resuming sensitive conversation."
                ));
            }
            Err(e) => {
                return Err(format!(
                    "Signal session check failed for {to}: {e}. \
                     The session may be corrupted; re-establish from Friend → Reset Secure Session \
                     after verifying their safety number out-of-band."
                ));
            }
        }
    } else {
        payload_bytes
    };

    // Build signed envelope
    let secret_key = {
        let sk = state.identity_secret.lock();
        *sk.as_ref().ok_or("signing key not initialized")?
    };

    let timestamp = rekindle_utils::timestamp_ms();

    let nonce = {
        let mut buf = [0u8; 16];
        rand::thread_rng().fill_bytes(&mut buf);
        buf.to_vec()
    };

    let envelope = build_envelope_from_secret(&secret_key, timestamp, nonce, final_payload);

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

    if let Err(e) = send_envelope(&routing_context, route_id, &envelope).await {
        // Invalidate the stale cached route so the next retry fetches fresh from DHT
        {
            let mut dht_mgr = state.dht_manager.write();
            if let Some(mgr) = dht_mgr.as_mut() {
                mgr.manager.invalidate_route_for_peer(to);
            }
        }
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
