use std::sync::Arc;

use tauri::AppHandle;

use crate::state::AppState;
use crate::state_helpers;

use rekindle_transport::operations::direct::NAK;

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

    // A 1:1 call (the `'D'` tag) is routed first, by its tag: the Host's
    // direct dispatch; an unported kind (a relay offer) is answered by the
    // desktop's arm (plan E2.3). Gossip never travels by `app_call`. The
    // untagged arms below keep their order until their owners tag them
    // (plan E2.3, "Ingress").
    match rekindle_codec::ingress::classify(&message) {
        rekindle_codec::ingress::Ingress::Direct(_) => {
            let reply = direct_call(state, &message).await;
            reply_to_call(state, call_id, reply).await;
            return;
        }
        rekindle_codec::ingress::Ingress::Gossip(_) => {
            tracing::debug!(msg_len = message.len(), "gossip by app_call — NAK");
            reply_to_call(state, call_id, NAK.to_vec()).await;
            return;
        }
        rekindle_codec::ingress::Ingress::Untagged => {}
    }

    // Cross-device sync envelopes carry their own JSON discriminator
    // (architecture §28.4 device pairing).
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

    let reply = community_call(app_handle, state, call_id, &message).await;
    reply_to_call(state, call_id, reply).await;
}

/// Send an `app_call`'s reply.
async fn reply_to_call(state: &Arc<AppState>, call_id: veilid_core::OperationId, reply: Vec<u8>) {
    if let Some(api) = state_helpers::veilid_api(state) {
        if let Err(e) = api.app_call_reply(call_id, reply).await {
            tracing::warn!(error = %e, "failed to reply to app_call");
        }
    }
}

/// Answer a community-envelope `app_call`.
async fn community_call(
    app_handle: &AppHandle,
    state: &Arc<AppState>,
    call_id: veilid_core::OperationId,
    message: &[u8],
) -> Vec<u8> {
    match rekindle_codec::capnp_envelope::try_decode_community_envelope(message) {
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
            // acknowledged. The sender is authenticated by its pseudonym
            // signature inside the payload (plan C7.29), not by an
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
            let owning_community = state.files.holder_of(attachment_uuid);
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
            tracing::debug!(msg_len = message.len(), "untagged app_call — NAK");
            NAK.to_vec()
        }
    }
}

/// Answer a 1:1 `app_call` through the Host: its reply (empty for a
/// handled call, whose return is the receipt, or a frame); an unported
/// kind is answered by the desktop's arm. With no host or no identity
/// database nothing was received, so the reply is the transport's [`NAK`],
/// which no caller takes for a receipt.
async fn direct_call(state: &Arc<AppState>, message: &[u8]) -> Vec<u8> {
    let Some(host) = state.host.current() else {
        return NAK.to_vec();
    };
    match host.receive_direct_call(message).await {
        rekindle_transport::operations::direct::Called::Answered(reply) => reply,
        rekindle_transport::operations::direct::Called::Unported(opened) => {
            let Ok(pool) = state.db.current() else {
                return NAK.to_vec();
            };
            super::inbound_unported::handle_call(state, &pool, *opened).await
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
    // The transport's readiness, which the Host's stages wait on (its login
    // stage and reconnect heal, plan E3.1 C2a/C2c): this shell runs the
    // node's update loop until E5.7.
    if let Some(transport) = state.transport.read().as_ref() {
        transport.shared().set_attachment(
            rekindle_transport::AttachmentState::from_veilid_string(&attachment.state.to_string()),
            attached,
            public_internet_ready,
        );
    }
    let state_str = attachment.state.to_string();
    tracing::info!(
        state = %state_str,
        public_internet_ready,
        "network attachment changed"
    );

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
    // so the friends' trigger below waits for both.
    let ready = public_internet_ready && attachment.live_peer_count.as_u64() >= 1;
    // The friends' network-ready trigger (plan E2.5): re-open their
    // watches and catch every chain up once the network is back.
    if let Some(host) = state.host.current() {
        host.friend_presence().network_changed(ready);
    }

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
    // The reconnect heal is the Host's login stage, on the transport's
    // readiness edge (plan E3.1 C2c).
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
        // The transport's decision, as the daemon's dispatch takes it
        // (`TransportNode::on_dead_remote_routes`). Voice resolves its routes through the same importer, so it holds
        // no copy of its own to evict.
        let dead: Vec<String> = change
            .dead_remote_routes
            .iter()
            .map(ToString::to_string)
            .collect();
        let affected = state_helpers::on_dead_remote_routes(state, &dead);
        if !affected.is_empty() {
            tracing::debug!(peers = affected.len(), "dropped dead remote routes");
        }
    }
}
