//! Inbound message dispatcher — the single entry point for all network data.
//!
//! Receives raw `VeilidUpdate` events from the node's update channel and
//! performs deterministic routing:
//!
//! 1. Parse frame header (4 bytes: version, type, length)
//! 2. Reject unknown versions (fail closed)
//! 3. Route by TypeId to the correct verification + deserialization path
//! 4. Verify Ed25519 signature (every message, no exceptions)
//! 5. Dedup check (gossip only)
//! 6. Forward gossip to mesh peers (if TTL > 0 and not private)
//! 7. Invoke the appropriate `InboundHandler` method with authenticated data
//!
//! If any step fails, the message is dropped and logged. No partial dispatch.

pub use crate::handler::TransportEvent;

use std::sync::Arc;

use tokio::sync::mpsc;
use tracing::{info, trace, warn};
use veilid_core::VeilidUpdate;

use crate::config::TransportConfig;
use crate::crypto::envelope::{Addressing, SignedPayload};
use crate::frame::{self, TypeId};
use crate::gossip::DedupCache;
use crate::handler::{InboundHandler, VerifiedSender};
use crate::shared::{AttachmentState, SharedState};

/// The route owners the dispatch loop reports `RouteChange` to.
pub(crate) struct RouteOwners {
    /// Our own routes.
    pub own: Arc<
        rekindle_protocol::own_routes::OwnRoutes<
            rekindle_protocol::own_routes::VeilidRouteAllocator,
        >,
    >,
    /// The process's importer of peers' routes.
    pub imports: Arc<rekindle_protocol::dht::route_imports::RouteImports>,
    /// The peers' cached route blobs.
    pub peers: Arc<parking_lot::RwLock<crate::broadcast::peer_registry::PeerRegistry>>,
}

/// Run the inbound dispatch loop until a shutdown signal is received.
pub(crate) async fn run_dispatch_loop<H: InboundHandler>(
    handler: Arc<H>,
    config: Arc<TransportConfig>,
    mut update_rx: mpsc::Receiver<VeilidUpdate>,
    mut shutdown_rx: mpsc::Receiver<()>,
    api: veilid_core::VeilidAPI,
    shared: Arc<SharedState>,
    routes: RouteOwners,
) {
    let mut dedup = DedupCache::new(config.dedup_cache_capacity);
    info!("transport dispatch loop started");

    loop {
        tokio::select! {
            Some(update) = update_rx.recv() => {
                dispatch_update(&handler, &config, &mut dedup, &api, &shared, &routes, update).await;
            }
            _ = shutdown_rx.recv() => {
                info!("transport dispatch loop shutting down");
                break;
            }
        }
    }
}

async fn dispatch_update<H: InboundHandler>(
    handler: &Arc<H>,
    config: &TransportConfig,
    dedup: &mut DedupCache,
    api: &veilid_core::VeilidAPI,
    shared: &SharedState,
    routes: &RouteOwners,
    update: VeilidUpdate,
) {
    match update {
        VeilidUpdate::AppMessage(msg) => {
            dispatch_app_message(handler, config, dedup, msg.message(), api, shared).await;
        }
        VeilidUpdate::AppCall(call) => {
            dispatch_app_call(handler, config, api, &call).await;
        }
        VeilidUpdate::ValueChange(change) => {
            dispatch_value_change(handler, shared, &change).await;
        }
        VeilidUpdate::Attachment(attachment) => {
            let state_str = attachment.state.to_string();
            let attached = attachment.state.is_attached();
            let pir = attachment.public_internet_ready;
            let att_state = AttachmentState::from_veilid_string(&state_str);
            shared.set_attachment(att_state, attached, pir);
            // 0.5.7's richer attachment signal — recorded so readiness
            // checks and status surfaces can use real peer counts instead
            // of inferring health from the attachment enum alone.
            shared.set_network_health(
                attachment.reliable_peer_count.as_u64(),
                attachment.live_peer_count.as_u64(),
                attachment.estimated_network_size.as_u64(),
                attachment
                    .median_latency
                    .map(veilid_core::TimestampDuration::as_u64)
                    .unwrap_or_default(),
            );
            handler
                .on_event(TransportEvent::AttachmentChanged {
                    state: state_str,
                    is_attached: attached,
                    public_internet_ready: pir,
                })
                .await;
        }
        VeilidUpdate::RouteChange(change) => {
            dispatch_route_change(handler, routes, &change).await;
        }
        VeilidUpdate::Shutdown => {
            info!("veilid shutdown event received");
        }
        _ => {
            trace!("ignoring unhandled VeilidUpdate variant");
        }
    }
}

async fn dispatch_app_message<H: InboundHandler>(
    handler: &Arc<H>,
    _config: &TransportConfig,
    dedup: &mut DedupCache,
    raw: &[u8],
    _api: &veilid_core::VeilidAPI,
    shared: &SharedState,
) {
    let (type_id, payload) = match frame::decode(raw) {
        Ok(result) => result,
        Err(e) => {
            // Not a transport frame. Before dropping it, try the
            // unframed gossip format: a Cap'n Proto `SignedEnvelope`
            // straight on `app_message`, which is what the desktop's
            // mesh broadcast sends. Without this branch a desktop peer's
            // gossip never reaches a daemon member at all — PATH 2 of
            // the three-path model simply does not cross the tracks.
            //
            // Safe unframed because the envelope authenticates itself:
            // `verify_gossip_signed_envelope` checks an Ed25519
            // signature over `(community_id, sender_pseudonym,
            // envelope_bytes)` made with the sender's community
            // pseudonym. The frame's signature would be redundant with
            // a property the payload already carries — the same
            // argument that admits the bare MEK transfer in
            // `dispatch_app_call`, and unlike that one it does not turn
            // on the payload type, so every variant is covered.
            if super::dispatch_gossip::dispatch_bare_gossip(handler, dedup, raw).await {
                return;
            }
            warn!(error = %e, raw_len = raw.len(), "dropping: frame decode failed");
            return;
        }
    };

    match type_id {
        // `TypeId::GossipBroadcast` is no longer produced. Gossip is the
        // unframed Cap'n Proto `SignedEnvelope` handled above, matching
        // the desktop; the framed postcard form existed only on this
        // track and no peer could read it. Falls through to the
        // "unexpected type" arm if one ever arrives.
        tid if !tid.is_rpc() => {
            dispatch_dm(handler, tid, payload, shared).await;
        }
        other => {
            warn!(
                type_id = other as u8,
                "unexpected RPC type in app_message, dropping"
            );
        }
    }
}

const APP_CALL_HANDLER_DEADLINE: std::time::Duration = std::time::Duration::from_secs(12);

async fn dispatch_app_call<H: InboundHandler>(
    handler: &Arc<H>,
    _config: &TransportConfig,
    api: &veilid_core::VeilidAPI,
    call: &veilid_core::VeilidAppCall,
) {
    let call_id = call.id();
    let raw = call.message();

    let (type_id, payload) = match frame::decode(raw) {
        Ok(r) => r,
        Err(e) => {
            // Not a transport frame. Before dropping it, check for the
            // one unframed format we deliberately accept: a bare Cap'n
            // Proto `CommunityEnvelope` carrying a wrapped MEK. That is
            // what the desktop track's rotator sends
            // (`services/veilid/network.rs`), and without this branch a
            // desktop rotator could never deliver a rotated key to a
            // daemon member — they would each NAK the other's format.
            if let Some(transfer) = super::bare_envelope::decode_bare_mek_transfer(raw) {
                dispatch_bare_mek_transfer(handler, api, call_id, transfer).await;
                return;
            }
            warn!(error = %e, "dropping app_call: frame decode failed");
            reply_nak(api, call_id).await;
            return;
        }
    };

    if !type_id.is_rpc() {
        warn!(
            type_id = type_id as u8,
            "non-RPC type in app_call, dropping"
        );
        reply_nak(api, call_id).await;
        return;
    }

    let signed: SignedPayload = match postcard::from_bytes(payload) {
        Ok(s) => s,
        Err(e) => {
            warn!(error = %e, "dropping app_call: deserialization failed");
            reply_nak(api, call_id).await;
            return;
        }
    };

    let Some(me) = handler.local_identity() else {
        warn!("dropping app_call: no unlocked identity");
        reply_nak(api, call_id).await;
        return;
    };
    let to_me = Addressing {
        recipient: &me,
        type_id,
    };
    if let Err(e) = crate::crypto::envelope::verify_signed_payload(&signed, to_me) {
        warn!(error = %e, sender = %signed.sender_key_hex, "dropping app_call: bad signature");
        reply_nak(api, call_id).await;
        return;
    }

    let request = match crate::payload::rpc::deserialize_inbound_call(type_id, &signed.payload) {
        Ok(r) => r,
        Err(e) => {
            warn!(error = %e, "dropping app_call: payload parse failed");
            reply_nak(api, call_id).await;
            return;
        }
    };

    let sender = if signed.sender_key_hex.is_empty() {
        None
    } else {
        Some(signed.sender_key_hex.as_str())
    };

    let response_bytes = if let Ok(bytes) = tokio::time::timeout(APP_CALL_HANDLER_DEADLINE, async {
        let response = handler.on_call(sender, request).await;
        crate::payload::rpc::serialize_call_response(&response)
    })
    .await
    {
        bytes
    } else {
        warn!(
            type_id = type_id as u8,
            deadline_secs = APP_CALL_HANDLER_DEADLINE.as_secs(),
            "app_call handler exceeded deadline — sending NAK"
        );
        b"NAK".to_vec()
    };

    if let Err(e) = api.app_call_reply(call_id, response_bytes).await {
        warn!(error = %e, "failed to send app_call reply");
    }
}

/// Hand a bare MEK transfer to the handler and reply in the same
/// dialect it arrived in.
///
/// The reply is a Cap'n Proto `MekTransferAck` rather than a serialized
/// `CallResponse`, because the sender is a desktop rotator awaiting
/// exactly that: `distribute_mek` inspects the reply for an ack whose
/// generation matches what it sent. A `CallResponse` there reads as a
/// delivery failure and makes the rotator log a mismatch on a transfer
/// that actually succeeded.
async fn dispatch_bare_mek_transfer<H: InboundHandler>(
    handler: &Arc<H>,
    api: &veilid_core::VeilidAPI,
    call_id: veilid_core::OperationId,
    transfer: rekindle_protocol::dht::community::envelope::MekTransferPayload,
) {
    use rekindle_protocol::dht::community::envelope::{
        CommunityEnvelope, ControlPayload, MekTransferAckPayload,
    };

    let community_id = transfer.community_id.clone();
    let channel_id = transfer.channel_id.clone();
    let generation = transfer.generation;
    // The sender is authenticated by the ECDH wrap, not by an envelope
    // signature, so it is passed through as the claimed sender and the
    // handler's unwrap is what actually decides.
    let sender = transfer.sender_pseudonym.clone();

    let response_bytes = match tokio::time::timeout(
        APP_CALL_HANDLER_DEADLINE,
        handler.on_call(
            Some(sender.as_str()),
            crate::payload::rpc::InboundCall::CommunityMekTransfer(transfer),
        ),
    )
    .await
    {
        // `Ok(bytes)` carries our own pseudonym hex — the handler is the
        // only layer that knows which pseudonym this community maps to,
        // and the rotator traces it to confirm *who* acked.
        Ok(crate::payload::rpc::CallResponse::Ok(requester)) => {
            let ack =
                CommunityEnvelope::Control(ControlPayload::MekTransferAck(MekTransferAckPayload {
                    community_id,
                    channel_id,
                    generation,
                    requester_pseudonym: String::from_utf8_lossy(&requester).into_owned(),
                }));
            rekindle_protocol::capnp_envelope::encode_community_envelope(&ack).unwrap_or_else(|e| {
                // The rotator is waiting on this reply. A bare NAK is a
                // worse answer than nothing, but it at least resolves
                // their `app_call` instead of leaving it to time out.
                warn!(error = %e, "encoding MekTransferAck failed");
                b"NAK".to_vec()
            })
        }
        Ok(other) => {
            warn!(
                community = %community_id,
                response = ?other,
                "MEK transfer not accepted"
            );
            b"NAK".to_vec()
        }
        Err(_) => {
            warn!(
                community = %community_id,
                deadline_secs = APP_CALL_HANDLER_DEADLINE.as_secs(),
                "MEK transfer handler exceeded deadline — sending NAK"
            );
            b"NAK".to_vec()
        }
    };

    if let Err(e) = api.app_call_reply(call_id, response_bytes).await {
        warn!(error = %e, "failed to send MEK transfer reply");
    }
}

async fn dispatch_dm<H: InboundHandler>(
    handler: &Arc<H>,
    type_id: TypeId,
    payload: &[u8],
    _shared: &SharedState,
) {
    let signed: SignedPayload = match postcard::from_bytes(payload) {
        Ok(s) => s,
        Err(e) => {
            warn!(error = %e, type_id = type_id as u8, "dropping DM: deserialization failed");
            return;
        }
    };

    let Some(me) = handler.local_identity() else {
        warn!(type_id = type_id as u8, "dropping DM: no unlocked identity");
        return;
    };
    let to_me = Addressing {
        recipient: &me,
        type_id,
    };
    if let Err(e) = crate::crypto::envelope::verify_signed_payload(&signed, to_me) {
        warn!(error = %e, sender = %signed.sender_key_hex, "dropping DM: bad signature");
        return;
    }

    let dm_payload = match crate::payload::dm::deserialize_dm(type_id, &signed.payload) {
        Ok(p) => p,
        Err(e) => {
            warn!(error = %e, type_id = type_id as u8, "dropping DM: payload parse failed");
            return;
        }
    };

    let sender = VerifiedSender {
        public_key: signed.sender_key_hex,
        display_name: String::new(),
    };

    // W16.7 — pass through seq + correlation_id from the wire envelope
    // so the implementer can run SeqTracker dedup before processing.
    // The fields are inside the Ed25519 signature scope (W16.3), so a
    // peer can't forge them after the fact.
    handler
        .on_dm(
            &sender,
            dm_payload,
            signed.timestamp,
            signed.seq,
            signed.correlation_id.as_deref(),
        )
        .await;
}

async fn dispatch_value_change<H: InboundHandler>(
    handler: &Arc<H>,
    shared: &SharedState,
    change: &veilid_core::VeilidValueChange,
) {
    let key = change.key.to_string();
    let subkeys: Vec<u32> = change.subkeys.iter().collect();
    let first_value = change.value.as_ref().map(|v| v.data().to_vec());
    // The pool is the one owner of watch death (plan C7.8): it re-arms the
    // watch of a record it holds, and a record it does not hold was
    // released, so its watch ends by design.
    if let Some(pool) = shared.records() {
        pool.on_value_change(change);
    }

    // A watch's last notification (`count == 0`) can still carry changes.
    if !subkeys.is_empty() {
        handler.on_value_change(&key, subkeys, first_value).await;
    }
}

async fn dispatch_route_change<H: InboundHandler>(
    handler: &Arc<H>,
    routes: &RouteOwners,
    change: &veilid_core::VeilidRouteChange,
) {
    if !change.dead_routes.is_empty() {
        // Ours among the dead are forgotten (never released: Veilid already
        // dropped them) and reallocated by their owner, off this loop.
        routes.own.on_dead(&change.dead_routes);
        let count = change.dead_routes.len();
        handler
            .on_event(TransportEvent::LocalRoutesDied { count })
            .await;
    }
    if !change.dead_remote_routes.is_empty() {
        // The importer forgets them (Veilid already released them) and the
        // peers holding one of those blobs lose it, so their next send
        // fetches a fresh route (plan C7.9e).
        let blobs = routes.imports.on_dead_remote(&change.dead_remote_routes);
        let peer_keys = routes.peers.write().invalidate_blobs(&blobs);
        handler
            .on_event(TransportEvent::RemoteRoutesDied { peer_keys })
            .await;
    }
}

async fn reply_nak(api: &veilid_core::VeilidAPI, call_id: veilid_core::OperationId) {
    if let Err(e) = api.app_call_reply(call_id, b"NAK".to_vec()).await {
        warn!(error = %e, "failed to send NAK reply");
    }
}
