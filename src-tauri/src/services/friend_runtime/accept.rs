//! Phase 23.C — split from friend_runtime.rs. accept_request orchestration.

use std::sync::Arc;

use crate::channels::ChatEvent;
use crate::db::{self, DbPool};
use crate::db_helpers::db_call;
use crate::services;
use crate::state::{AppState, FriendState, FriendshipState, UserStatus};
use crate::state_helpers;

use super::{auto_volunteer_relay_enabled, read_pending_request_data};

/// Accept a pending friend request: read stored bundle/route/invite
/// data, INSERT/DELETE in friends + pending_friend_requests, cache
/// route blob, install FriendState, persist DHT keys, establish
/// initiator-side Signal session (with W16.10e idempotency + W16.10d
/// never-silent error emit), send friend-accept via Veilid, watch
/// the friend's profile DHT, mark invite accepted, auto-volunteer
/// relay if enabled, emit FriendRequestAccepted, append FriendAdded
/// audit-chain entry.
pub async fn accept_request_inner(
    state: Arc<AppState>,
    pool: DbPool,
    app: tauri::AppHandle,
    public_key: String,
    display_name: String,
) -> Result<(), String> {
    let owner_key = state_helpers::current_owner_key(&state)?;
    let timestamp = db::timestamp_now();

    let (
        pending_profile_key,
        pending_mailbox_key,
        pending_route_blob,
        pending_prekey_bundle,
        pending_invite_id,
    ) = read_pending_request_data(&pool, &owner_key, &public_key).await?;

    let pk = public_key.clone();
    let dn = display_name.clone();
    let ok = owner_key.clone();
    db_call(&pool, move |conn| {
        conn.execute(
            "INSERT OR IGNORE INTO friends (owner_key, public_key, display_name, added_at) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![ok, pk, dn, timestamp],
        )?;
        conn.execute(
            "DELETE FROM pending_friend_requests WHERE owner_key = ?1 AND public_key = ?2",
            rusqlite::params![ok, pk],
        )?;
        Ok(())
    })
    .await?;

    if let Some(ref blob) = pending_route_blob {
        if !blob.is_empty() {
            let api = state_helpers::veilid_api(&state);
            if let Some(api) = api {
                let mut dht_mgr = state.dht_manager.write();
                if let Some(mgr) = dht_mgr.as_mut() {
                    mgr.manager.cache_route(&api, &public_key, blob.clone());
                }
            }
        }
    }

    let friend = FriendState {
        public_key: public_key.clone(),
        display_name: display_name.clone(),
        nickname: None,
        status: UserStatus::Offline,
        status_message: None,
        game_info: None,
        group: None,
        unread_count: 0,
        dht_record_key: pending_profile_key.clone(),
        last_seen_at: None,
        local_conversation_key: None,
        remote_conversation_key: None,
        mailbox_dht_key: pending_mailbox_key.clone(),
        last_heartbeat_at: None,
        friendship_state: FriendshipState::Accepted,
    };
    state.friends.write().insert(public_key.clone(), friend);

    if pending_profile_key.is_some() || pending_mailbox_key.is_some() {
        let pk3 = public_key.clone();
        let ok3 = state_helpers::current_owner_key(&state)?;
        let pdk = pending_profile_key.clone();
        let mdk = pending_mailbox_key;
        db_call(&pool, move |conn| {
            crate::friend_repo::update_dht_and_mailbox_keys(
                conn,
                &ok3,
                &pk3,
                pdk.as_deref(),
                mdk.as_deref(),
            )
        })
        .await?;
    }

    let session_init = if let Some(ref prekey_bytes) = pending_prekey_bundle {
        let signal = state.signal_manager.read();
        if let Some(handle) = signal.as_ref() {
            let bundle =
                match serde_json::from_slice::<rekindle_crypto::signal::PreKeyBundle>(prekey_bytes)
                {
                    Ok(b) => Some(b),
                    Err(e) => {
                        tracing::error!(peer = %public_key, error = %e,
                            "failed to deserialize stored prekey bundle — cannot establish Signal session");
                        let peer_label = state_helpers::friend_display_name(&state, &public_key)
                            .unwrap_or_else(|| format!("{}…", &public_key[..16.min(public_key.len())]));
                        crate::event_dispatch::emit_live(
                            &app,
                            "notification-event",
                            &crate::channels::NotificationEvent::SystemAlert {
                                title: "Couldn't establish secure session".into(),
                                body: format!(
                                    "Stored prekey bundle for {peer_label} is unparseable. \
                                     Tell them to re-send the friend request, then verify their \
                                     safety number out-of-band before accepting again."
                                ),
                            },
                        );
                        None
                    }
                };

            match bundle {
                None => None,
                Some(bundle) => {
                    let already_established = handle
                        .manager
                        .has_session(&public_key)
                        .unwrap_or(false)
                        && handle
                            .manager
                            .is_trusted_identity(&public_key, &bundle.identity_key)
                            .unwrap_or(false);
                    if already_established {
                        tracing::info!(peer = %public_key,
                            "session already established for peer — skipping establish_session \
                             (W16.10e idempotency)");
                        None
                    } else {
                        match handle.manager.establish_session(&public_key, &bundle) {
                            Ok(info) => {
                                tracing::info!(peer = %public_key,
                                    "established initiator Signal session on accept");
                                Some(info)
                            }
                            Err(e) => {
                                tracing::error!(peer = %public_key, error = %e,
                                    "failed to establish Signal session on accept — recipient won't receive ephemeral_key, encrypted DMs to them will fail AEAD on their side");
                                let peer_label = state_helpers::friend_display_name(&state, &public_key)
                                    .unwrap_or_else(|| format!("{}…", &public_key[..16.min(public_key.len())]));
                                crate::event_dispatch::emit_live(
                                    &app,
                                    "notification-event",
                                    &crate::channels::NotificationEvent::SystemAlert {
                                        title: "Couldn't establish secure session".into(),
                                        body: format!(
                                            "Failed to establish encrypted session with {peer_label}: {e}. \
                                             Their side will not receive the session-init data; \
                                             subsequent encrypted messages will fail. \
                                             Tell them to re-send the friend request, then verify their \
                                             safety number out-of-band before accepting again."
                                        ),
                                    },
                                );
                                None
                            }
                        }
                    }
                }
            }
        } else {
            None
        }
    } else {
        tracing::debug!(peer = %public_key, "no stored prekey bundle for session establishment");
        None
    };

    services::message_service::send_friend_accept(&state, &pool, &public_key, session_init)
        .await
        .unwrap_or_else(|e| {
            tracing::warn!(error = %e, "failed to send friend accept via Veilid");
        });

    if let Some(ref dht_key) = pending_profile_key {
        if let Err(e) = services::presence_service::watch_friend(&state, &public_key, dht_key).await
        {
            tracing::trace!(error = %e, "failed to watch friend DHT after accepting request");
        }
    }

    if let Some(ref iid) = pending_invite_id {
        let ok = state_helpers::current_owner_key(&state).unwrap_or_default();
        crate::invite_helpers::mark_invite_accepted(&pool, &ok, iid);
    }

    if auto_volunteer_relay_enabled(&app) {
        if let Err(e) =
            crate::services::relay::offer::volunteer_relay(&state, &pool, &public_key).await
        {
            tracing::debug!(
                friend = %public_key,
                error = %e,
                "auto-volunteer Strand Relay route failed (friendship still accepted)"
            );
        }
    }

    crate::event_dispatch::emit_live(
        &app,
        "chat-event",
        &ChatEvent::FriendRequestAccepted {
            from: public_key.clone(),
            display_name: display_name.clone(),
        },
    );

    crate::audit_repo::append_async(
        &state,
        &pool,
        &owner_key,
        rekindle_audit::AuditKind::FriendAdded,
        serde_json::json!({
            "peer_public_key": public_key,
            "display_name": display_name,
            "direction": "inbound",
        }),
    )
    .await;

    Ok(())
}

