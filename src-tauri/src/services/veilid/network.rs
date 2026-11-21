use std::sync::Arc;

use tauri::AppHandle;

use crate::state::AppState;
use crate::state_helpers;

use rekindle_transport::liveness::{Liveness, ProbeAnswer};
use rekindle_transport::operations::direct::NAK;

/// The adopted transport node's route liveness (plan RH1r-c), which this
/// shell's update loop feeds until E5.7: our probes, arrivals over our
/// routes and the network's return.
pub(super) fn route_liveness(state: &Arc<AppState>) -> Option<Arc<Liveness>> {
    state
        .transport
        .read()
        .as_ref()
        .map(|transport| Arc::clone(transport.liveness()))
}

use rekindle_codec::community::envelope::{CommunityEnvelope, ControlPayload};

pub async fn handle_app_call(
    app_handle: &AppHandle,
    state: &Arc<AppState>,
    call: veilid_core::VeilidAppCall,
) {
    let call_id = call.id();
    tracing::debug!(call_id = %call_id, "app_call received");

    let message = call.message().to_vec();
    let liveness = route_liveness(state);
    if let Some(live) = &liveness {
        // A liveness probe (plan RH1r-c O1): ours echoes, another's is
        // refused, as the daemon's dispatch answers it
        // (`rekindle-transport` `subscriptions::dispatch`).
        match live.answer_probe(&message) {
            ProbeAnswer::NotProbe => {}
            ProbeAnswer::Echo(echo) => {
                reply_to_call(state, call_id, echo).await;
                return;
            }
            ProbeAnswer::Refuse => {
                reply_to_call(state, call_id, NAK.to_vec()).await;
                return;
            }
        }
        // Arrival over our route proves its inbound path (O1).
        live.heard_over(call.route_id().map(ToString::to_string).as_deref());
    }

    // A 1:1 call (the `'D'` tag) is routed first, by its tag: the Host's
    // direct dispatch; an unported kind (a relay offer) is answered by the
    // desktop's arm (plan E2.3). Gossip never travels by `app_call`. The
    // untagged arms below keep their order until their owners tag them
    // (plan E2.3, "Ingress").
    match rekindle_codec::ingress::classify(&message) {
        rekindle_codec::ingress::Ingress::Direct(_) => {
            // A call left unanswered gets no reply: the caller's `app_call`
            // times out and is attempted again (a NAK is a refusal, RV2).
            if let Some(reply) = direct_call(state, &message).await {
                reply_to_call(state, call_id, reply).await;
            }
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

    if let Some(reply) = community_call(app_handle, state, call_id, &message).await {
        reply_to_call(state, call_id, reply).await;
    }
}

/// Send an `app_call`'s reply.
async fn reply_to_call(state: &Arc<AppState>, call_id: veilid_core::OperationId, reply: Vec<u8>) {
    if let Some(api) = state_helpers::veilid_api(state) {
        if let Err(e) = api.app_call_reply(call_id, reply).await {
            tracing::warn!(error = %e, "failed to reply to app_call");
        }
    }
}

/// Answer a community-envelope `app_call`; `None` leaves it unanswered (a
/// partial community's bootstrap, plan E3.2 S7c.8 (1)).
async fn community_call(
    app_handle: &AppHandle,
    state: &Arc<AppState>,
    call_id: veilid_core::OperationId,
    message: &[u8],
) -> Option<Vec<u8>> {
    Some(
        match rekindle_codec::capnp_envelope::try_decode_community_envelope(message) {
            Ok(Some(CommunityEnvelope::Control(ControlPayload::BootstrapRequest {
                joiner_pseudonym,
                anchor,
                member_joined,
                member_joined_op,
                joiner_route,
            }))) => {
                let joiner = rekindle_governance_runtime::BootstrapJoiner {
                    pseudonym: joiner_pseudonym,
                    member_joined,
                    member_joined_op,
                    route: joiner_route,
                };
                match crate::services::community::answer_bootstrap_request(
                    app_handle, state, &anchor, &joiner,
                )
                .await
                {
                    Ok(response) => response,
                    Err(e) => {
                        tracing::warn!(call_id = %call_id, error = %e, "bootstrap request refused");
                        NAK.to_vec()
                    }
                }
            }
            Ok(Some(CommunityEnvelope::Control(ControlPayload::CheckpointStateRequest {
                request,
            }))) => {
                // A member's state snapshot request (plan E3.2 S7c.3): the Host
                // admits and rate-limits the signer; a refusal is a NAK.
                match state.host.current() {
                    Some(host) => host
                        .governance()
                        .answer_checkpoint_state(&request)
                        .await
                        .unwrap_or_else(|| NAK.to_vec()),
                    None => NAK.to_vec(),
                }
            }
            Ok(Some(CommunityEnvelope::Control(ControlPayload::CheckpointHintRequest {
                request,
            }))) => {
                // A member's or a joiner's checkpoint hint request (plan E3.2
                // S7c.5): admitted and rate-limited like a snapshot request.
                match state.host.current() {
                    Some(host) => host
                        .governance()
                        .answer_checkpoint_hint(&request)
                        .await
                        .unwrap_or_else(|| NAK.to_vec()),
                    None => NAK.to_vec(),
                }
            }
            Ok(Some(CommunityEnvelope::Control(ControlPayload::MekTransfer(transfer)))) => {
                // The Host's one receive path (plan E3.1, CB3 glue audit), the
                // daemon's too: membership and removal checks, unwrap, install
                // (catch-up included), `MekRotated`, and the `MekTransferAck`
                // whose key-confirmation tag proves the unwrap (P1.3, review
                // S7m part 1). A refusal is a bare `ACK`: no tag, so the
                // sender does not settle the transfer.
                match state_helpers::host_governance(state) {
                    Ok(host) => host
                        .receive_mek_transfer(&transfer)
                        .unwrap_or_else(|refusal| {
                            tracing::warn!(
                                call_id = %call_id,
                                reason = refusal.reason(),
                                "MEK transfer refused — replying bare ACK"
                            );
                            b"ACK".to_vec()
                        }),
                    Err(e) => {
                        tracing::warn!(call_id = %call_id, error = %e, "MEK transfer while locked — replying bare ACK");
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
                .await
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
                rekindle_node::host::adapters::files::ops::serve(
                    &state.files,
                    attachment_id,
                    &requested_chunks,
                )
                .unwrap_or_else(|| b"ACK".to_vec())
            }
            _ => {
                tracing::debug!(msg_len = message.len(), "untagged app_call — NAK");
                NAK.to_vec()
            }
        },
    )
}

/// Answer a 1:1 `app_call` through the Host: its reply (empty for a
/// handled call, whose return is the receipt, or a frame); an unported
/// kind is answered by the desktop's arm. A call the Host could not judge
/// (its deadline, an internal error) is answered retry later
/// (`direct::RETRY_LATER`, review RV5 M2); with no host there is no reply.
/// Either way the caller attempts it again.
async fn direct_call(state: &Arc<AppState>, message: &[u8]) -> Option<Vec<u8>> {
    let host = state.host.current()?;
    match host.receive_direct_call(message).await {
        rekindle_transport::operations::direct::Called::Answered(reply) => Some(reply),
        rekindle_transport::operations::direct::Called::RetryLater => {
            Some(rekindle_transport::operations::direct::RETRY_LATER.to_vec())
        }
        rekindle_transport::operations::direct::Called::Unported(opened) => {
            let pool = state.db.current().ok()?;
            Some(super::inbound_unported::handle_call(state, &pool, *opened).await)
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
    // Veilid 0.5.8 re-sends an unchanged attachment after an executor stall
    // whose tick lag exceeds TICK_LAG_REPORT (2 s) and is at least half the
    // RPC timeout (attachment_manager/mod.rs:24-25,557-581,
    // online_detector.rs:63,711), and to an app back from the background
    // (attachment_manager/mod.rs:578-580,655-661), so readiness watchers wake
    // only on a real edge (synthesis #6).
    state.network_ready.send_if_modified(|ready| {
        let changed = *ready != public_internet_ready;
        *ready = public_internet_ready;
        changed
    });
    // The transport's readiness, which the Host's stages wait on (its login
    // stage and reconnect heal, plan E3.1 C2a/C2c): this shell runs the
    // node's update loop until E5.7.
    if let Some(transport) = state.transport.read().as_ref() {
        // The network came back: our routes are tested at once (O5), as
        // the daemon's dispatch does on the same edge.
        let shared = transport.shared();
        if (attached && !shared.is_attached())
            || (public_internet_ready && !shared.public_internet_ready())
        {
            transport.liveness().network_changed();
        }
        transport.shared().set_attachment(
            rekindle_transport::AttachmentState::from_veilid_string(&attachment.state.to_string()),
            attached,
            public_internet_ready,
        );
    }
    let state_str = attachment.state.to_string();
    super::sample_strength(app_handle, state, &state_str);
    tracing::info!(
        state = %state_str,
        public_internet_ready,
        "network attachment changed"
    );

    let attached_edge = {
        let mut guard = state.node.write();
        // No node handle yet: the first update is the edge.
        guard.as_mut().is_none_or(|node| {
            let edge = node.is_attached != attached;
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
            edge
        })
    };
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

    // The OS notification is for an attached/detached edge only: an
    // unchanged re-send after a stall (above) raises none.
    if !attached_edge {
        return;
    }
    let status = if attached {
        "connected"
    } else {
        "disconnected"
    };
    let notification = rekindle_types::subscription_events::NotificationEvent::SystemAlert {
        title: "Network".to_string(),
        subject: None,
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
