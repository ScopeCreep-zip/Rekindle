use std::sync::Arc;

use tauri::AppHandle;

use crate::services::{message_service, sync_service};
use crate::state::AppState;
use crate::state_helpers;

use rekindle_codec::community::envelope::{
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
        let Ok(pool) = state.db.current() else {
            tracing::debug!("inbound app_call: no identity database — dropped");
            return;
        };
        let reply = match crate::services::cross_device_sync::handle_pairing_app_call(
            state, &pool, payload,
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

    let reply_bytes = match rekindle_codec::capnp_envelope::try_decode_community_envelope(&message)
    {
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
            let received = rekindle_types::channel_keys::KeyScope::from_wire(channel_id.as_deref())
                .ok_or_else(|| format!("MekTransfer names no key scope: {channel_id:?}"))
                .and_then(|scope| {
                    crate::services::community::handle_incoming_mek_transfer(
                        app_handle,
                        state,
                        &community_id,
                        scope,
                        &sender_pseudonym,
                        &wrapped_mek,
                    )
                });
            match received {
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
                    rekindle_codec::capnp_envelope::encode_community_envelope(&ack).unwrap_or_else(
                        |e| {
                            tracing::warn!(
                                call_id = %call_id,
                                error = %e,
                                "failed to encode MekTransferAck — falling back to bare ACK"
                            );
                            b"ACK".to_vec()
                        },
                    )
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
        Ok(Some(CommunityEnvelope::Control(
            payload @ (ControlPayload::VoiceMediaKey { .. }
            | ControlPayload::VoiceMediaKeyRequest { .. }),
        ))) => {
            // Plan C7.22 — call-media keys arrive point to point and are
            // acknowledged. The sender is authenticated by the HPKE Auth
            // seal (as a MEK transfer is by its ECDH wrap), not by an
            // envelope signature.
            crate::services::voice_signaling_adapter::answer_media_key_call(
                app_handle, state, &payload,
            )
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
            let Ok(pool) = state.db.current() else {
                tracing::debug!("inbound app_call: no identity database — dropped");
                return;
            };
            // Architecture §27.1: a DmInvite arriving via app_call must
            // get a structured DmAccept/DmDecline reply, not a bare
            // ACK. Try that path first; fall through to the generic
            // handler for everything else.
            if let Some(reply) =
                message_service::try_handle_dm_invite_app_call(app_handle, state, &pool, &message)
                    .await
            {
                reply
            } else {
                message_service::handle_incoming_message(app_handle, state, &pool, &message).await;
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
    state.network_ready.send_replace(public_internet_ready);
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
        let state = state.clone();
        let app_handle = app_handle.clone();
        crate::state_helpers::login_scope_or_closed(&state).spawn_or_drop(
            "attachment heal",
            async move {
                // A route that died while detached was reported dead and is
                // being reallocated by its owner (plan C7.9b).
                crate::services::governance_adapter::open_community_dht_records(&state).await;
                crate::services::governance_adapter::rebuild_governance_from_dht(&state).await;
                let _ = sync_service::sync_friends_now(&state, &app_handle).await;
            },
        );
    }
}

pub fn handle_route_change(state: &Arc<AppState>, change: &veilid_core::VeilidRouteChange) {
    tracing::debug!(
        dead_routes = change.dead_routes.len(),
        dead_remote_routes = change.dead_remote_routes.len(),
        "route change event"
    );

    // Our own routes among the dead are forgotten (never released: Veilid
    // already dropped them) and reallocated by their owner, off this
    // receive loop (plan C7.9b).
    if let Some(routes) = state_helpers::own_routes(state) {
        routes.on_dead(&change.dead_routes);
    }
    // A relay route among them is re-volunteered with a fresh offer to its
    // friend (plan C7.9f).
    if !change.dead_routes.is_empty() {
        if let Ok(pool) = state.db.current() {
            let state_c = Arc::clone(state);
            let dead = change.dead_routes.clone();
            state_helpers::spawn_in_login(state, "relay route re-volunteer", async move {
                crate::services::relay::offer::on_dead_relay_routes(&state_c, &pool, &dead).await;
            });
        }
    }

    if !change.dead_remote_routes.is_empty() {
        // The importer forgets the dead routes (Veilid already dropped
        // them, so nothing is released) and the peers whose cached blob was
        // one of them lose it, so the next send re-fetches a fresh route.
        // Voice resolves its routes through the same importer, so it holds
        // no copy of its own to evict.
        let affected = state_helpers::on_dead_remote_routes(state, &change.dead_remote_routes);
        if !affected.is_empty() {
            tracing::debug!(peers = affected.len(), "dropped dead remote routes");
        }
    }
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
