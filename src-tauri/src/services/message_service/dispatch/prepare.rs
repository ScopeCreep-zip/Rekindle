//! Envelope verification + Signal decrypt + block-list + non-friend
//! filter. Run before any handler fans out the `MessagePayload`. Used
//! by `dispatch::mod::handle_incoming_message` and by the `app_call`
//! reply path (`try_handle_dm_invite_app_call`).

use std::sync::Arc;

use rekindle_protocol::messaging::envelope::MessagePayload;
use rekindle_protocol::messaging::receiver::{parse_payload, process_incoming};

use crate::db::DbPool;
use crate::db_helpers::db_call_or_default;
use crate::state::AppState;
use crate::state_helpers;

use super::PreparedMessage;

/// Parse envelope, check block list, decrypt, deserialize payload, and filter non-friends.
///
/// Returns `None` (with appropriate logging) for any rejection reason.
pub(super) async fn prepare_incoming(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    pool: &DbPool,
    raw_message: &[u8],
) -> Option<PreparedMessage> {
    let envelope = match process_incoming(raw_message) {
        Ok(env) => env,
        Err(e) => {
            tracing::error!(error = %e, "failed to parse/verify incoming message envelope");
            return None;
        }
    };

    let sender_hex = hex::encode(&envelope.sender_key);
    tracing::debug!(from = %sender_hex, payload_len = envelope.payload.len(), "processing verified envelope");

    if is_blocked(state, pool, &sender_hex).await {
        tracing::debug!(from = %sender_hex, "dropping message from blocked user");
        return None;
    }

    let payload_bytes = decrypt_payload(state, app_handle, &sender_hex, &envelope.payload).await?;

    let payload = match parse_payload(&payload_bytes) {
        Ok(p) => p,
        Err(e) => {
            tracing::error!(error = %e, from = %sender_hex, "failed to parse message payload");
            return None;
        }
    };

    // Phase 2 Track A — primary receive-path authorization gate consults
    // SQLite via FriendStore (source of truth), not the in-memory
    // state.friends map. Fixes the dispatch-before-hydration race that
    // caused "not friends even though we are" AEAD failures.
    let is_friend_payload = matches!(
        payload,
        MessagePayload::FriendRequest { .. }
            | MessagePayload::FriendRequestReceived
            | MessagePayload::Unfriended
            | MessagePayload::UnfriendedAck
            | MessagePayload::FriendReject
            | MessagePayload::RelayEnvelope { .. }
            | MessagePayload::DmInvite { .. }
            | MessagePayload::GroupDmInvite { .. }
            | MessagePayload::WakeNotify { .. }
    );
    if !is_friend_payload
        && !state_helpers::is_active_friend_authoritative(state, &sender_hex).await
    {
        tracing::debug!(from = %sender_hex, "dropping message from non-friend");
        return None;
    }

    let ts: i64 = envelope.timestamp.try_into().unwrap_or(i64::MAX);
    Some(PreparedMessage {
        sender_hex,
        payload,
        timestamp: ts,
    })
}

/// Attempt Signal decryption; pass through if already valid JSON (plaintext).
///
/// Returns `None` on decrypt failure (after emitting a notification to the frontend)
/// or if no Signal manager is available for a non-JSON payload.
async fn decrypt_payload(
    state: &Arc<AppState>,
    app_handle: &tauri::AppHandle,
    sender_hex: &str,
    raw_payload: &[u8],
) -> Option<Vec<u8>> {
    if serde_json::from_slice::<serde_json::Value>(raw_payload).is_ok() {
        return Some(raw_payload.to_vec());
    }

    // Phase 6 — clone the Arc out of the read-guard so we can `.await`
    // on the now-async decrypt without holding a parking_lot guard
    // across the await (the guard is !Send).
    let handle = state
        .signal_manager
        .read()
        .as_ref()
        .map(std::sync::Arc::clone);
    if let Some(handle) = handle {
        match handle.manager.decrypt(sender_hex, raw_payload).await {
            Ok(pt) => Some(pt),
            Err(e) => {
                // W16.10d — was warn, now error so RUST_LOG=info catches it.
                tracing::error!(
                    error = %e, from = %sender_hex,
                    payload_len = raw_payload.len(),
                    "encrypted message could not be decrypted (Signal AEAD failure — most likely cause: responder-side respond_to_session failed during friend-add, see prior 'Couldn't establish secure session' alerts)"
                );
                let display_name = state_helpers::friend_display_name(state, sender_hex);
                let from_label = display_name
                    .unwrap_or_else(|| format!("{}...", &sender_hex[..8.min(sender_hex.len())]));
                crate::event_dispatch::emit_live(
                    app_handle,
                    "notification-event",
                    &crate::channels::NotificationEvent::SystemAlert {
                        title: "Message Decrypt Failed".to_string(),
                        body: format!(
                            "A message from {from_label} could not be decrypted. The secure \
                             session is missing or corrupted on this side. Open their friend \
                             menu and click 'Reset Secure Session' — verify their safety \
                             number out-of-band before re-establishing."
                        ),
                    },
                );
                None
            }
        }
    } else {
        tracing::warn!(from = %sender_hex, "received non-JSON payload but no signal manager");
        None
    }
}

async fn is_blocked(state: &Arc<AppState>, pool: &DbPool, sender_hex: &str) -> bool {
    let owner_key = state_helpers::owner_key_or_default(state);
    let pk = sender_hex.to_string();
    db_call_or_default(pool, move |conn| {
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM blocked_users WHERE owner_key = ?1 AND public_key = ?2",
            rusqlite::params![owner_key, pk],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    })
    .await
}
