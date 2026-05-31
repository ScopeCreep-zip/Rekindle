//! `FriendRequest` / `FriendAccept` full receive handlers — they
//! orchestrate the session installer (`session.rs`), the SQLite
//! persist, the friend-list state mutation, the responder-side
//! `FriendRequestReceived` ACK, and the cross-request auto-accept
//! short-circuit (delegated to `super::lifecycle::auto_accept_cross_request`).

use std::sync::Arc;

use rekindle_protocol::messaging::envelope::MessagePayload;

use crate::channels::ChatEvent;
use crate::db::DbPool;
use crate::db_helpers::db_call;
use crate::state::AppState;
use crate::state_helpers;

use super::lifecycle::{auto_accept_cross_request, delete_pending_request_row};
use super::session::{handle_friend_accept, handle_friend_request};
use super::{IncomingFriendAccept, IncomingFriendRequest};
use crate::services::message_service::{
    build_and_queue_envelope, send_friend_reject, send_to_peer_raw,
};

pub(crate) async fn handle_friend_request_full(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    pool: &DbPool,
    req: &IncomingFriendRequest<'_>,
) {
    handle_friend_request(req.sender_hex, req.prekey_bundle);

    // Cache the sender's route blob for immediate replies
    tracing::info!(
        from = %req.sender_hex,
        route_blob_len = req.route_blob.len(),
        route_count = req.route_blob.first().copied().unwrap_or(0),
        "handle_friend_request_full: received peer route blob"
    );
    if !req.route_blob.is_empty() {
        state_helpers::cache_peer_route(state, req.sender_hex, req.route_blob.to_vec());
    }

    // If sender is already in our friend list, check for cross-request auto-accept
    let existing_friendship_state =
        state_helpers::friend_field(state, req.sender_hex, |f| Some(f.friendship_state));

    if let Some(fs) = existing_friendship_state {
        if fs == crate::state::FriendshipState::PendingOut {
            // Cross-request: both parties want the friendship — auto-accept
            tracing::info!(from = %req.sender_hex, "cross-request detected — auto-accepting");
            auto_accept_cross_request(app_handle, state, pool, req).await;
            return;
        }
        if fs == crate::state::FriendshipState::Accepted {
            // W16.10e (fix A) — receive-side dedup. The previous behavior
            // (silent wipe + re-prompt the user as a fresh incoming request)
            // matched the comment's "Briar re-add = new contact" intent
            // but missed Briar's actual receive-side dedup at the BSP
            // layer. With our existing retry queue (sync_service @ 30s),
            // network duplicates of the same FriendRequest are routine —
            // and every duplicate was destroying the working session.
            //
            // Pattern matches SimpleX's `withInvLock c (strEncode inv)` +
            // `case conn` discriminator (Agent.hs ~L1900): an Active
            // duplex connection refuses retry mutations; only the
            // bundle's identity_key change (genuine peer re-onboard)
            // requires explicit user consent.
            //
            // libsignal's analog is `IdentityKeyStore::is_trusted_identity`
            // TOFU (IdentityKeyStore.java:54-60): identity matches stored
            // → safe to proceed; mismatch → require explicit user trust
            // via Direction::SENDING UntrustedIdentityException.
            let bundle_identity_key =
                serde_json::from_slice::<rekindle_crypto::signal::PreKeyBundle>(req.prekey_bundle)
                    .ok()
                    .map(|b| b.identity_key);

            let identity_matches = if let Some(ref ik) = bundle_identity_key {
                let signal = state.signal_manager.read();
                signal
                    .as_ref()
                    .and_then(|h| h.manager.is_trusted_identity(req.sender_hex, ik).ok())
                    .unwrap_or(false)
            } else {
                false
            };

            if identity_matches {
                // Network retry — peer is the same identity, just re-sending
                // because their FriendRequestReceived ACK never landed
                // (or the retry queue is in flight). Re-send the ACK
                // synchronously to silence their retry; do NOT wipe the
                // friendship, do NOT prompt the user, do NOT touch the
                // Signal session.
                tracing::info!(
                    from = %req.sender_hex,
                    "FriendRequest from already-Accepted peer with matching identity — \
                     treating as retry, re-sending ACK"
                );
                if let Err(e) = send_to_peer_raw(
                    state,
                    pool,
                    req.sender_hex,
                    &MessagePayload::FriendRequestReceived,
                )
                .await
                {
                    tracing::debug!(
                        to = %req.sender_hex,
                        error = %e,
                        "failed to re-send FriendRequestReceived ACK on retry — sender's \
                         retry queue will fire again later"
                    );
                }
                return;
            }

            // Identity mismatch — peer re-onboarded with new keys, OR
            // someone is impersonating. Per the vulnerable-user safety
            // stance (`feedback_vulnerable_users_no_creative_paths.md`):
            // never silently wipe / never auto-trust new keys / always
            // require explicit user consent verified against an
            // out-of-band safety number. Surface as a SystemAlert
            // pointing at the existing Reset Secure Session path; the
            // user's friendship + session stay untouched until they
            // explicitly confirm.
            let peer_label = state_helpers::friend_display_name(state, req.sender_hex)
                .unwrap_or_else(|| format!("{}…", &req.sender_hex[..16.min(req.sender_hex.len())]));
            tracing::error!(
                from = %req.sender_hex,
                "FriendRequest from already-Accepted peer with DIFFERENT identity_key — \
                 leaving existing friendship intact, asking user to verify and reset"
            );
            crate::event_dispatch::emit_live(
                app_handle,
                "notification-event",
                &crate::channels::NotificationEvent::SystemAlert {
                    title: "Peer's identity key has changed".into(),
                    body: format!(
                        "{peer_label} sent a friend request with a new identity key. \
                         This usually means they re-installed the app, but it could also \
                         be an impersonation attempt. Verify their safety number out-of-band \
                         (phone call, in person), then click 'Reset Secure Session' from \
                         their friend menu to accept the new keys."
                    ),
                },
            );
            return;
        }
        if fs == crate::state::FriendshipState::Removing {
            // Removing: user explicitly initiated removal. Clear stale
            // state and treat the incoming request as a fresh contact.
            state.friends.write().remove(req.sender_hex);
            crate::friend_repo::fire_delete_friend(state, pool, req.sender_hex);
            delete_pending_request_row(state, pool, req.sender_hex);
            tracing::info!(
                from = %req.sender_hex,
                "received friend request from Removing peer — treating as new request"
            );
        } else {
            // PendingIn or other unexpected state — just update display name
            {
                let mut friends = state.friends.write();
                if let Some(friend) = friends.get_mut(req.sender_hex) {
                    friend.display_name = req.display_name.to_string();
                }
            }
            crate::friend_repo::fire_update_display_name(
                state,
                pool,
                req.sender_hex,
                req.display_name,
            );
            return;
        }
    }

    // Invite correlation: if this request carries an invite_id, check if cancelled
    if let Some(iid) = req.invite_id {
        let owner_key = state_helpers::owner_key_or_default(state);
        if crate::invite_helpers::is_invite_cancelled(pool, &owner_key, iid).await {
            tracing::info!(from = %req.sender_hex, %iid, "rejecting request for cancelled invite");
            let _ = send_friend_reject(state, pool, req.sender_hex).await;
            return;
        }
        crate::invite_helpers::mark_invite_responded(pool, &owner_key, iid, req.sender_hex);
    }

    // B5/P3.1 — persist BEFORE emit so the DB row exists by the time
    // chat-event reaches the frontend. Crash between emit-and-persist
    // (the prior db_fire spawn behavior) left a phantom request in memory
    // that vanished on restart and could never be accepted.
    if let Err(e) = persist_friend_request(state, pool, req).await {
        tracing::warn!(
            from = %req.sender_hex,
            error = %e,
            "failed to persist friend request — skipping event emit and ACK to avoid phantom UI state"
        );
        return;
    }
    let event = ChatEvent::FriendRequest {
        from: req.sender_hex.to_string(),
        display_name: req.display_name.to_string(),
        message: req.message.to_string(),
    };
    crate::event_dispatch::emit_journaled(app_handle, state, "chat-event", &event);

    // B10/P3.4 — try the ACK send immediately; if it fails (peer offline,
    // route stale, app_message rejected), queue for retry through the
    // sync_service pending_messages loop. The previous `let _ = ...` swallow
    // meant the sender never learned we received their friend request and
    // kept retrying it forever from their side, eventually appearing
    // duplicate-spammy in the receiver's buddy list. The queue is bounded
    // (20 retries × 30s = 10 minutes per the existing sync_service drop
    // policy) so this can't loop forever.
    if let Err(e) = send_to_peer_raw(
        state,
        pool,
        req.sender_hex,
        &MessagePayload::FriendRequestReceived,
    )
    .await
    {
        tracing::info!(
            to = %req.sender_hex,
            error = %e,
            "FriendRequestReceived ACK send failed, queueing for sync_service retry"
        );
        if let Err(e) = build_and_queue_envelope(
            state,
            pool,
            req.sender_hex,
            &MessagePayload::FriendRequestReceived,
        )
        .await
        {
            tracing::warn!(
                to = %req.sender_hex,
                error = %e,
                "failed to queue FriendRequestReceived ACK for retry — sender may keep re-sending the request"
            );
        }
    }
}

/// Handle a `FriendAccept` with profile key, route blob, and mailbox key exchange.
pub(crate) async fn handle_friend_accept_full(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    pool: &DbPool,
    a: &IncomingFriendAccept<'_>,
) {
    // Guard: ignore FriendAccept if we are in the process of removing this friend
    let is_removing = state_helpers::friend_field(state, a.sender_hex, |f| {
        Some(matches!(
            f.friendship_state,
            crate::state::FriendshipState::Removing
        ))
    })
    .unwrap_or(false);
    if is_removing {
        tracing::info!(from = %a.sender_hex, "ignoring FriendAccept — friend is being removed");
        return;
    }

    handle_friend_accept(
        app_handle,
        state,
        a.sender_hex,
        a.prekey_bundle,
        a.ephemeral_key,
        a.signed_prekey_id,
        a.one_time_prekey_id,
        a.ml_kem_ciphertext,
        a.used_ot_pqpk_id,
    );
    // Cache the acceptor's route blob
    if !a.route_blob.is_empty() {
        state_helpers::cache_peer_route(state, a.sender_hex, a.route_blob.clone());
    }
    // Store profile key, mailbox key, and transition friendship to Accepted
    {
        let mut friends = state.friends.write();
        if let Some(friend) = friends.get_mut(a.sender_hex) {
            if !a.profile_dht_key.is_empty() {
                friend.dht_record_key = Some(a.profile_dht_key.to_string());
            }
            if !a.mailbox_dht_key.is_empty() {
                friend.mailbox_dht_key = Some(a.mailbox_dht_key.to_string());
            }
            friend.friendship_state = crate::state::FriendshipState::Accepted;
        }
    }
    // Persist friendship_state transition to DB
    crate::friend_repo::fire_update_friendship_state(state, pool, a.sender_hex, "accepted");
    // Persist profile key to `SQLite`
    if !a.profile_dht_key.is_empty() {
        crate::friend_repo::fire_update_dht_record_key(
            state,
            pool,
            a.sender_hex,
            a.profile_dht_key,
        );
        // Start watching the friend's profile DHT record for presence
        if let Err(e) =
            crate::services::presence_service::watch_friend(state, a.sender_hex, a.profile_dht_key)
                .await
        {
            tracing::trace!(from = %a.sender_hex, error = %e, "failed to watch friend after accept");
        }
    }
    let display_name = state_helpers::friend_display_name(state, a.sender_hex)
        .unwrap_or_else(|| a.sender_hex.to_string());
    let event = ChatEvent::FriendRequestAccepted {
        from: a.sender_hex.to_string(),
        display_name,
    };
    crate::event_dispatch::emit_journaled(app_handle, state, "chat-event", &event);
}

async fn persist_friend_request(
    state: &Arc<AppState>,
    pool: &DbPool,
    req: &IncomingFriendRequest<'_>,
) -> Result<(), String> {
    let owner_key = state_helpers::owner_key_or_default(state);
    let pk = req.sender_hex.to_string();
    let dn = req.display_name.to_string();
    let msg = req.message.to_string();
    let pdk = req.profile_dht_key.to_string();
    let rb = req.route_blob.to_vec();
    let mdk = req.mailbox_dht_key.to_string();
    let pkb = req.prekey_bundle.to_vec();
    let iid = req.invite_id.map(str::to_string);
    let now = crate::db::timestamp_now();
    db_call(pool, move |conn| {
        conn.execute(
            "INSERT OR REPLACE INTO pending_friend_requests \
             (owner_key, public_key, display_name, message, received_at, profile_dht_key, route_blob, mailbox_dht_key, prekey_bundle, invite_id) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            rusqlite::params![owner_key, pk, dn, msg, now, pdk, rb, mdk, pkb, iid],
        )?;
        Ok(())
    })
    .await
}
