//! Envelope verification (recipient, freshness, replay) + sealing check +
//! Signal decrypt + block-list + non-friend filter. Run before any handler
//! fans out the `MessagePayload`. Used by
//! `dispatch::mod::handle_incoming_message` and by the `app_call` reply
//! path (`try_handle_dm_invite_app_call`).

use std::sync::Arc;

use rekindle_codec::message::envelope::{MessagePayload, Sealing};
use rekindle_protocol::messaging::receiver::{
    parse_sealed_payload, process_incoming, received_sealing,
};
use rekindle_protocol::messaging::replay::Sighting;
use rekindle_types::friend_store::FriendshipState;

use crate::db_helpers::db_call;
use crate::state::AppState;
use crate::state_helpers;
use rekindle_db::Db;

use super::PreparedMessage;

/// Parse envelope, check block list, decrypt, deserialize payload, and filter non-friends.
///
/// Returns `None` (with appropriate logging) for any rejection reason.
pub(super) async fn prepare_incoming(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    pool: &Db,
    raw_message: &[u8],
) -> Option<PreparedMessage> {
    let my_key = match state_helpers::current_owner_key(state)
        .and_then(|k| super::super::seal::recipient_key(&k))
    {
        Ok(key) => key,
        Err(e) => {
            tracing::debug!(error = %e, "dropping envelope: no local identity");
            return None;
        }
    };
    let now = rekindle_utils::timestamp_ms();
    let envelope = match process_incoming(raw_message, &my_key, now) {
        Ok(env) => env,
        Err(e) => {
            tracing::warn!(error = %e, "dropping incoming envelope");
            return None;
        }
    };

    let sender_hex = hex::encode(&envelope.sender_key);
    match state.envelope_replay.lock().observe(
        &envelope.sender_key,
        &envelope.nonce,
        envelope.timestamp,
        now,
    ) {
        Sighting::First => {}
        Sighting::Duplicate => {
            tracing::debug!(from = %sender_hex, "dropping duplicate envelope");
            return None;
        }
        Sighting::Full => {
            tracing::warn!(from = %sender_hex, "replay guard full — dropping envelope");
            return None;
        }
    }
    tracing::debug!(from = %sender_hex, payload_len = envelope.payload.len(), "processing verified envelope");

    match is_blocked(state, pool, &sender_hex).await {
        Ok(false) => {}
        Ok(true) => {
            tracing::debug!(from = %sender_hex, "dropping message from blocked user");
            return None;
        }
        Err(e) => {
            // Fail closed: a sender whose block state is unknown is not let in.
            tracing::warn!(from = %sender_hex, error = %e, "block list unreadable; dropping message");
            return None;
        }
    }

    let received = received_sealing(&envelope.payload);
    let opened = match received {
        Sealing::Plain => envelope.payload.clone(),
        Sealing::Session => {
            decrypt_payload(state, app_handle, &sender_hex, &envelope.payload).await?
        }
    };
    let payload = match parse_sealed_payload(&opened, received) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(error = %e, from = %sender_hex, "dropping message payload");
            return None;
        }
    };

    // The receive gate reads friendship status from SQLite via FriendStore
    // (the source of truth), not the in-memory `state.friends` map, so a
    // message dispatched before hydration is judged on the stored row.
    let status = state_helpers::friend_status_authoritative(state, &sender_hex).await;
    if !sender_admitted(&payload, status) {
        tracing::info!(from = %sender_hex, ?status, kind = payload_kind(&payload),
            "dropping message: sender not admitted for this payload");
        return None;
    }

    let ts: i64 = envelope.timestamp.try_into().unwrap_or(i64::MAX);
    Some(PreparedMessage {
        sender_hex,
        payload,
        timestamp: ts,
    })
}

/// Whether a sender with friendship `status` may deliver `payload`.
///
/// - The friend-handshake and invitation payloads come from anyone not
///   blocked (blocking is checked before this).
/// - `FriendAccept` comes from a sender we sent a request to
///   (`PendingOut`) or one already `Accepted`: the higher side of a crossing
///   request is `Accepted` with no session when the lower side's accept
///   arrives, and `handle_friend_accept` decides whether to answer it.
///   Never from a stranger or a `Removing` row. Before plan C7.25 it
///   needed `Accepted`, so the requester dropped every accept.
/// - Everything else needs `Accepted`.
pub(super) fn sender_admitted(payload: &MessagePayload, status: Option<FriendshipState>) -> bool {
    match payload {
        MessagePayload::FriendRequest { .. }
        | MessagePayload::FriendRequestReceived
        | MessagePayload::Unfriended
        | MessagePayload::UnfriendedAck
        | MessagePayload::FriendReject
        | MessagePayload::RelayEnvelope { .. }
        | MessagePayload::DmInvite { .. }
        | MessagePayload::GroupDmInvite { .. }
        | MessagePayload::WakeNotify { .. } => true,
        MessagePayload::FriendAccept { .. } => {
            matches!(
                status,
                Some(FriendshipState::PendingOut | FriendshipState::Accepted)
            )
        }
        _ => status == Some(FriendshipState::Accepted),
    }
}

/// The payload's variant name, for the drop log.
fn payload_kind(payload: &MessagePayload) -> &'static str {
    match payload {
        MessagePayload::FriendAccept { .. } => "FriendAccept",
        MessagePayload::DirectMessage { .. } => "DirectMessage",
        MessagePayload::SessionResetRequest { .. } => "SessionResetRequest",
        MessagePayload::SessionResetAccept { .. } => "SessionResetAccept",
        _ => "other",
    }
}

/// Signal-decrypt a session-sealed payload.
///
/// Returns `None` on decrypt failure (after emitting a notification to the frontend)
/// or if no Signal manager is available.
async fn decrypt_payload(
    state: &Arc<AppState>,
    app_handle: &tauri::AppHandle,
    sender_hex: &str,
    raw_payload: &[u8],
) -> Option<Vec<u8>> {
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
                let from_label = display_name.unwrap_or_else(|| {
                    format!("{}...", rekindle_utils::text::prefix(sender_hex, 8))
                });
                crate::event_dispatch::emit_notification(
                    app_handle,
                    rekindle_types::subscription_events::NotificationEvent::SystemAlert {
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
        tracing::warn!(from = %sender_hex, "received session-sealed payload but no signal manager");
        None
    }
}

async fn is_blocked(state: &Arc<AppState>, pool: &Db, sender_hex: &str) -> Result<bool, String> {
    let owner_key = state_helpers::owner_key_or_default(state);
    let pk = sender_hex.to_string();
    db_call(pool, move |conn| {
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM blocked_users WHERE owner_key = ?1 AND public_key = ?2",
            rusqlite::params![owner_key, pk],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn accept() -> MessagePayload {
        MessagePayload::FriendAccept {
            prekey_bundle: Vec::new(),
            profile_dht_key: String::new(),
            route_blob: Vec::new(),
            mailbox_dht_key: String::new(),
            ephemeral_key: Vec::new(),
            signed_prekey_id: 1,
            one_time_prekey_id: None,
            ml_kem_ciphertext: Vec::new(),
            used_ot_pqpk_id: None,
        }
    }

    fn dm() -> MessagePayload {
        MessagePayload::DirectMessage {
            body: "hi".into(),
            reply_to: None,
        }
    }

    #[test]
    fn friend_accept_reaches_the_requester() {
        assert!(sender_admitted(
            &accept(),
            Some(FriendshipState::PendingOut)
        ));
        // The higher side of a crossing request is already Accepted.
        assert!(sender_admitted(&accept(), Some(FriendshipState::Accepted)));
    }

    #[test]
    fn friend_accept_from_a_stranger_or_a_removed_row_drops() {
        assert!(!sender_admitted(&accept(), None));
        assert!(!sender_admitted(&accept(), Some(FriendshipState::Removing)));
    }

    #[test]
    fn a_pending_sender_may_send_nothing_but_the_accept() {
        assert!(!sender_admitted(&dm(), Some(FriendshipState::PendingOut)));
        assert!(!sender_admitted(&dm(), None));
        assert!(sender_admitted(&dm(), Some(FriendshipState::Accepted)));
    }
}
