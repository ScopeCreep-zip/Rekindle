//! `attempt_pending_retry` body — parses the row's JSON body as a DM
//! `MessageEnvelope` and sends it. Channel writes are not queued here: a
//! missed channel write is held and re-pushed by the record pool and saved
//! to `dht_outbox` at logout (plan C7.13). Lifted out of
//! `deps_impl.rs` so each trait method body stays one-liner thin.
//!
//! Returns the [`PendingRetryOutcome`] verbatim — the orchestrator
//! decides what to do with it (`Delivered` → delete row, `Failed`
//! → bump retry, `Unrecognized` → drop).

use std::sync::Arc;

use rekindle_codec::message::envelope::MessageEnvelope;
use rekindle_sync::{PendingMessageRow, PendingRetryOutcome};

use crate::state::AppState;
use crate::state_helpers;

pub(super) async fn attempt_pending_retry(
    state: &Arc<AppState>,
    row: &PendingMessageRow,
    stop: &tokio_util::sync::CancellationToken,
) -> PendingRetryOutcome {
    if let Ok(envelope) = serde_json::from_str::<MessageEnvelope>(&row.body) {
        return attempt_dm_retry(state, &row.recipient_key, &envelope, stop).await;
    }
    PendingRetryOutcome::Unrecognized
}

/// Retry one pending DM envelope: import the cached route, fall back to a
/// mailbox-DHT read on miss, re-sign with a fresh timestamp, send via
/// `messaging::sender::send_envelope`. Session end interrupts the mailbox
/// read and the send; the row stays for the next session.
async fn attempt_dm_retry(
    state: &Arc<AppState>,
    recipient_key: &str,
    envelope: &MessageEnvelope,
    stop: &tokio_util::sync::CancellationToken,
) -> PendingRetryOutcome {
    let route_id_and_rc = state_helpers::try_import_peer_route(state, recipient_key);
    if route_id_and_rc.is_none() && state_helpers::safe_api_and_routing_context(state).is_none() {
        // Not attached yet — bump retry, try again next tick.
        return PendingRetryOutcome::Failed;
    }
    let route_id_and_rc = if route_id_and_rc.is_some() {
        route_id_and_rc
    } else {
        match stop
            .run_until_cancelled(try_mailbox_route_fallback(state, recipient_key))
            .await
        {
            Some(found) => found,
            None => return PendingRetryOutcome::Interrupted,
        }
    };
    let Some((route_id, routing_context)) = route_id_and_rc else {
        return PendingRetryOutcome::Failed;
    };
    // Fresh timestamp, same nonce: the receiver's freshness window would
    // reject the original signature, and a copy that already arrived is
    // dropped as a duplicate.
    let envelope = match crate::services::message_service::resign_for_retry(
        state,
        recipient_key,
        envelope,
    ) {
        Ok(envelope) => envelope,
        Err(error) => {
            tracing::warn!(to = %recipient_key, %error, "pending DM cannot be re-signed — dropping");
            return PendingRetryOutcome::Unrecognized;
        }
    };
    let Some(sent) = stop
        .run_until_cancelled(rekindle_protocol::messaging::sender::send_envelope(
            &routing_context,
            route_id.clone(),
            &envelope,
        ))
        .await
    else {
        return PendingRetryOutcome::Interrupted;
    };
    match state_helpers::note_send_result(state, &route_id, sent) {
        Ok(()) => {
            tracing::debug!(to = %recipient_key, "pending DM delivered successfully");
            PendingRetryOutcome::Delivered
        }
        Err(error) => {
            tracing::debug!(to = %recipient_key, %error, "pending DM retry failed");
            PendingRetryOutcome::Failed
        }
    }
}

async fn try_mailbox_route_fallback(
    state: &Arc<AppState>,
    recipient_key: &str,
) -> Option<(veilid_core::RouteId, veilid_core::RoutingContext)> {
    let mailbox_key = state_helpers::friend_mailbox_key(state, recipient_key)?;
    let rc = state_helpers::safe_routing_context(state)?;
    let record_pool = state_helpers::record_pool(state).ok()?;
    let route_blob =
        match rekindle_protocol::dht::mailbox::read_peer_mailbox_route(&record_pool, &mailbox_key)
            .await
        {
            Ok(Some(blob)) if !blob.is_empty() => blob,
            Ok(_) => return None,
            Err(error) => {
                tracing::trace!(to = %recipient_key, %error, "failed to read mailbox");
                return None;
            }
        };
    state_helpers::cache_peer_route(state, recipient_key, route_blob.clone());
    match state_helpers::import_route_blob(state, &route_blob) {
        Ok(route_id) => {
            tracing::debug!(to = %recipient_key, "discovered route via mailbox fallback");
            Some((route_id, rc))
        }
        Err(error) => {
            tracing::trace!(to = %recipient_key, %error, "failed to import mailbox route");
            None
        }
    }
}
