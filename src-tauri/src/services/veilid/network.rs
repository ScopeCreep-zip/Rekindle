use std::sync::Arc;
use std::time::Instant;

use tauri::{AppHandle, Manager};

use crate::db::DbPool;
use crate::services::{message_service, sync_service};
use crate::state::AppState;
use crate::state_helpers;

use rekindle_protocol::dht::community::envelope::{
    CommunityEnvelope, ControlPayload, MekTransferAckPayload, MekTransferPayload,
};

fn resolve_bootstrap_community_id(state: &Arc<AppState>, governance_key: &str) -> Option<String> {
    let communities = state.communities.read();
    communities
        .values()
        .find(|community| community.governance_key.as_deref() == Some(governance_key))
        .map(|community| community.id.clone())
}

pub async fn handle_app_call(
    app_handle: &AppHandle,
    state: &Arc<AppState>,
    call: veilid_core::VeilidAppCall,
) {
    let call_id = call.id();
    tracing::debug!(call_id = %call_id, "app_call received");

    let message = call.message().to_vec();

    // Cross-device sync envelopes are tagged with their own discriminator,
    // so try them first (architecture §28.4 device pairing).
    if let Ok(rekindle_types::cross_device_sync::SyncEnvelope::PairingRequest(payload)) =
        serde_json::from_slice(&message)
    {
        let pool: tauri::State<'_, DbPool> = app_handle.state();
        let reply = match crate::services::cross_device_sync::handle_pairing_app_call(
            state,
            pool.inner(),
            payload,
        )
        .await
        {
            Ok(accept) => serde_json::to_vec(&accept).unwrap_or_else(|_| b"ACK".to_vec()),
            Err(e) => {
                tracing::warn!(call_id = %call_id, error = %e, "pairing app_call failed");
                b"ACK".to_vec()
            }
        };
        if let Some(api) = state_helpers::veilid_api(state) {
            if let Err(e) = api.app_call_reply(call_id, reply).await {
                tracing::warn!(error = %e, "failed to reply to pairing app_call");
            }
        }
        return;
    }

    let reply_bytes = match rekindle_protocol::capnp_envelope::try_decode_community_envelope(
        &message,
    ) {
        Ok(Some(CommunityEnvelope::Control(ControlPayload::BootstrapRequest {
            joiner_pseudonym,
            governance_key,
        }))) => {
            if let Some(community_id) = resolve_bootstrap_community_id(state, &governance_key) {
                crate::services::community::build_bootstrap_response(
                    state,
                    &community_id,
                    &governance_key,
                    &joiner_pseudonym,
                )
                .await
                .unwrap_or_else(|e| {
                    tracing::warn!(call_id = %call_id, error = %e, "failed to build bootstrap response");
                    b"ACK".to_vec()
                })
            } else {
                tracing::warn!(
                    call_id = %call_id,
                    governance_key = %governance_key,
                    "rejecting bootstrap request for unknown governance key"
                );
                b"ACK".to_vec()
            }
        }
        Ok(Some(CommunityEnvelope::Control(ControlPayload::MekTransfer(MekTransferPayload {
            community_id,
            channel_id,
            generation,
            sender_pseudonym,
            wrapped_mek,
        })))) => {
            // P1.3 — reply with a Cap'n-Proto-encoded `MekTransferAck`
            // instead of a bare `b"ACK"`. Confirms BOTH (1) the unwrap
            // succeeded at the app layer (a network-layer success
            // alone leaves the responder uncertain whether decryption
            // worked) and (2) the generation we ack'd matches what
            // they sent (catches misrouted app_calls).
            //
            // On unwrap failure we still reply `b"ACK"` so the
            // responder's app_call await resolves; the caller's
            // tracing surfaces the error path.
            match crate::services::community::handle_incoming_mek_transfer(
                app_handle,
                state,
                &community_id,
                channel_id.as_deref(),
                &sender_pseudonym,
                &wrapped_mek,
            ) {
                Ok(_) => {
                    let requester_pseudonym = {
                        let communities = state.communities.read();
                        communities
                            .get(&community_id)
                            .and_then(|cs| cs.my_pseudonym_key.clone())
                            .unwrap_or_default()
                    };
                    let ack = CommunityEnvelope::Control(ControlPayload::MekTransferAck(
                        MekTransferAckPayload {
                            community_id: community_id.clone(),
                            channel_id: channel_id.clone(),
                            generation,
                            requester_pseudonym,
                        },
                    ));
                    rekindle_protocol::capnp_envelope::encode_community_envelope(&ack)
                        .unwrap_or_else(|e| {
                            tracing::warn!(
                                call_id = %call_id,
                                error = %e,
                                "failed to encode MekTransferAck — falling back to bare ACK"
                            );
                            b"ACK".to_vec()
                        })
                }
                Err(error) => {
                    tracing::warn!(
                        call_id = %call_id,
                        error = %error,
                        "failed to handle incoming MEK transfer — replying bare ACK"
                    );
                    b"ACK".to_vec()
                }
            }
        }
        Ok(Some(CommunityEnvelope::Control(ControlPayload::RequestAttachment {
            channel_id: _,
            attachment_id,
            requested_chunks,
            requester_pseudonym,
        }))) => {
            // Find the community that owns this attachment by scanning each
            // open file cache for the requested attachment_id. The requester's
            // pseudonym is informational here — permission to read the file
            // is implicit in being able to decrypt the FEK (which only
            // community members can do).
            let _ = requester_pseudonym;
            let attachment_uuid = uuid::Uuid::from_bytes(attachment_id);
            let owning_community = {
                let caches = state.file_caches.read();
                caches.iter().find_map(|(cid, cache)| {
                    cache
                        .stats_per_attachment()
                        .contains_key(&attachment_uuid)
                        .then(|| cid.clone())
                })
            };
            match owning_community {
                Some(cid) => crate::services::community::files::serve_attachment_request(
                    state,
                    &cid,
                    attachment_id,
                    &requested_chunks,
                )
                .unwrap_or_else(|| b"ACK".to_vec()),
                None => b"ACK".to_vec(),
            }
        }
        _ => {
            let pool: tauri::State<'_, DbPool> = app_handle.state();
            // Architecture §27.1: a DmInvite arriving via app_call must
            // get a structured DmAccept/DmDecline reply, not a bare
            // ACK. Try that path first; fall through to the generic
            // handler for everything else.
            if let Some(reply) = message_service::try_handle_dm_invite_app_call(
                app_handle,
                state,
                pool.inner(),
                &message,
            )
            .await
            {
                reply
            } else {
                message_service::handle_incoming_message(app_handle, state, pool.inner(), &message)
                    .await;
                b"ACK".to_vec()
            }
        }
    };

    if let Some(api) = state_helpers::veilid_api(state) {
        if let Err(e) = api.app_call_reply(call_id, reply_bytes).await {
            tracing::warn!(error = %e, "failed to reply to app_call");
        }
    }
}

pub fn handle_attachment(
    app_handle: &AppHandle,
    state: &Arc<AppState>,
    attachment: &veilid_core::VeilidStateAttachment,
) {
    let attached = attachment.state.is_attached();
    let public_internet_ready = attachment.public_internet_ready;
    let state_str = attachment.state.to_string();
    tracing::info!(
        state = %state_str,
        public_internet_ready,
        "network attachment changed"
    );

    let was_attached = state_helpers::is_attached(state);
    let reconnected = !was_attached && attached;

    {
        if let Some(ref mut node) = *state.node.write() {
            node.attachment_state = state_str;
            node.is_attached = attached;
            node.public_internet_ready = public_internet_ready;
            // 0.5.7's richer attachment signal — real peer counts and
            // latency instead of inferring health from the enum alone.
            node.reliable_peer_count = attachment.reliable_peer_count.as_u64();
            node.live_peer_count = attachment.live_peer_count.as_u64();
            node.estimated_network_size = attachment.estimated_network_size.as_u64();
            node.median_latency_us = attachment
                .median_latency
                .map(veilid_core::TimestampDuration::as_u64)
                .unwrap_or_default();
        }
    }
    // Network readiness = the routing domain is up AND the routing table
    // holds at least one live peer. `public_internet_ready` in practice
    // implies bootstrap contact, so this rarely differs — but when it
    // does (domain flagged ready before any peer entry lands), DHT and
    // route work issued in that window fails with transient KeyNotFound,
    // which is exactly what `wait_for_network_ready` exists to prevent.
    let ready = public_internet_ready && attachment.live_peer_count.as_u64() >= 1;
    let _ = state.network_ready_tx.send(ready);

    super::emit_network_status(app_handle, state);

    let status = if attached {
        "connected"
    } else {
        "disconnected"
    };
    let notification = rekindle_types::subscription_events::NotificationEvent::SystemAlert {
        title: "Network".to_string(),
        body: format!("Veilid network {status}"),
    };
    crate::event_dispatch::emit_notification(app_handle, notification);

    if reconnected && public_internet_ready && state.identity.read().is_some() {
        tracing::info!(
            "network reconnected; invalidating watches, rebuilding governance, triggering friend resync"
        );
        invalidate_all_watches(state);
        let routeless = {
            let node = state.node.read();
            node.as_ref().is_some_and(|nh| nh.route_blob.is_none())
        };
        let state = state.clone();
        let app_handle = app_handle.clone();
        tokio::spawn(async move {
            // Heal the route FIRST — the governance/presence republish
            // below and every send path need a live inbound route.
            if routeless {
                allocate_fresh_private_route(&app_handle, &state).await;
            }
            crate::services::governance_adapter::open_community_dht_records(&state).await;
            crate::services::governance_adapter::rebuild_governance_from_dht(&state).await;
            let _ = sync_service::sync_friends_now(&state, &app_handle).await;
        });
    }
}

pub async fn handle_route_change(
    app_handle: &AppHandle,
    state: &Arc<AppState>,
    change: &veilid_core::VeilidRouteChange,
) {
    tracing::debug!(
        dead_routes = change.dead_routes.len(),
        dead_remote_routes = change.dead_remote_routes.len(),
        "route change event"
    );

    let our_route_died = {
        let rm = state.routing_manager.read();
        rm.as_ref().is_some_and(|handle| {
            handle
                .manager
                .route_id()
                .is_some_and(|our_id| change.dead_routes.contains(&our_id))
        })
    };

    if our_route_died {
        let heal_admitted = {
            let mut rm = state.routing_manager.write();
            match *rm {
                Some(ref mut handle) => {
                    handle.manager.forget_private_route();
                    let admitted = handle.heal_gate.try_begin(Instant::now());
                    // A8 telemetry: admitted/suppressed ratio is the
                    // baseline for retuning HEAL_COOLDOWN post-0.5.7.
                    if admitted {
                        handle.heal_attempts_admitted += 1;
                    } else {
                        handle.heal_attempts_suppressed += 1;
                    }
                    admitted
                }
                None => false,
            }
        };
        // The dead blob must not linger as Some — the routeless
        // watchdog keys off `route_blob.is_none()`.
        if let Some(ref mut nh) = *state.node.write() {
            nh.route_blob = None;
        }
        let attached = state_helpers::is_attached(state);
        if attached && heal_admitted {
            allocate_fresh_private_route(app_handle, state).await;
        } else {
            // Detached (reattach hook heals) or within the heal
            // cooldown during a flap (watchdog backstops within 30s).
            tracing::debug!(
                attached,
                heal_admitted,
                "dead route heal deferred (flap guard / detached)"
            );
        }
    }

    if !change.dead_remote_routes.is_empty() {
        let affected_pubkeys = {
            let mut dht_mgr = state.dht_manager.write();
            dht_mgr.as_mut().map_or_else(Vec::new, |mgr| {
                mgr.manager
                    .invalidate_dead_routes(&change.dead_remote_routes)
            })
        };
        // The DHTManager caches aren't the only copy: the live
        // peer-route cache holds the same blobs and must drop them too,
        // or sends keep importing a route Veilid just declared dead.
        if !affected_pubkeys.is_empty() {
            let mut rm = state.routing_manager.write();
            if let Some(ref mut handle) = *rm {
                for pubkey in &affected_pubkeys {
                    handle.peer_route_cache.remove(pubkey);
                }
            }
        }
    }
}

/// Route-allocation failure — distinguishes "Veilid rejected the
/// allocation" (transient: peerinfo / relay readiness can lag a route
/// death or network-ready signal by seconds) from "the API handle is
/// gone" (hard: logout/shutdown mid-retry — do not keep trying).
enum RouteAllocError {
    ApiGone,
    Veilid(veilid_core::VeilidAPIError),
}

impl std::fmt::Display for RouteAllocError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ApiGone => write!(f, "veilid API no longer available"),
            Self::Veilid(e) => write!(f, "{e}"),
        }
    }
}

/// Attempt `api.new_private_route()` up to `attempts` times with a fixed
/// 3-second delay — THE private-route allocation retry for the Tauri host
/// (login startup and dead-route healing both come through here). The API
/// handle is re-fetched from state on every attempt so a logout mid-retry
/// aborts instead of spinning. Built on [`rekindle_utils::retry`] — the
/// one retry loop shared across the workspace.
pub(crate) async fn new_private_route_with_retry(
    state: &Arc<AppState>,
    attempts: u32,
) -> Option<veilid_core::RouteBlob> {
    rekindle_utils::retry::retry_with_backoff(
        rekindle_utils::retry::RetryPolicy::fixed(attempts, std::time::Duration::from_secs(3)),
        "private-route-allocate",
        |e| matches!(e, RouteAllocError::Veilid(_)),
        || async {
            let api = state_helpers::veilid_api(state).ok_or(RouteAllocError::ApiGone)?;
            api.new_private_route()
                .await
                .map_err(RouteAllocError::Veilid)
        },
    )
    .await
    .map_err(|e| {
        tracing::warn!(attempts, error = %e, "private route allocation failed");
    })
    .ok()
}

pub(crate) async fn allocate_fresh_private_route(app_handle: &AppHandle, state: &Arc<AppState>) {
    let Some(route_blob) = new_private_route_with_retry(state, 5).await else {
        tracing::warn!(
            "dead-route recovery: all allocation attempts failed; refresh-loop backstop will retry"
        );
        return;
    };

    {
        let mut rm = state.routing_manager.write();
        if let Some(ref mut handle) = *rm {
            handle
                .manager
                .set_allocated_route(route_blob.route_id.clone(), route_blob.blob.clone());
        }
    }
    if let Some(ref mut nh) = *state.node.write() {
        nh.route_blob = Some(route_blob.blob.clone());
    }
    super::emit_network_status(app_handle, state);

    if let Err(e) = message_service::push_profile_update(state, 6, route_blob.blob.clone()).await {
        tracing::warn!(error = %e, "failed to re-publish route blob to DHT");
    }

    let mailbox_key = {
        let node = state.node.read();
        node.as_ref().and_then(|nh| nh.mailbox_dht_key.clone())
    };
    if let Some(mailbox_key) = mailbox_key {
        let rc = {
            let node = state.node.read();
            node.as_ref().map(|nh| nh.routing_context.clone())
        };
        if let Some(rc) = rc {
            if let Err(e) = rekindle_protocol::dht::mailbox::update_mailbox_route(
                &rc,
                &mailbox_key,
                &route_blob.blob,
            )
            .await
            {
                tracing::warn!(error = %e, "failed to update mailbox route blob");
            }
        }
    }

    // SMPL presence registry rows carry our route blob — targeted
    // re-write per joined community so gossip peers pick the new blob
    // up on their next poll/watch (replaces the old refresh-loop's
    // full rejoin + needs_initial_sync storm).
    let community_ids: Vec<String> = {
        let communities = state.communities.read();
        communities.keys().cloned().collect()
    };
    for community_id in &community_ids {
        crate::services::community::presence::registry::write_our_presence(state, community_id)
            .await;
    }

    // Dead-route recovery is the worst case for voice peers — the blob
    // they hold died with the route. Re-announce immediately.
    crate::services::voice_adapter::reannounce_voice_route(state);

    tracing::info!("re-allocated private route");
}

fn invalidate_all_watches(state: &Arc<AppState>) {
    let friend_keys: Vec<String> = {
        let friends = state.friends.read();
        friends
            .values()
            .filter(|f| f.dht_record_key.is_some())
            .map(|f| f.public_key.clone())
            .collect()
    };
    if !friend_keys.is_empty() {
        let mut unwatched = state.unwatched_friends.write();
        for key in &friend_keys {
            unwatched.insert(key.clone());
        }
        tracing::info!(
            count = friend_keys.len(),
            "invalidated all friend watches for re-establishment"
        );
    }
}
