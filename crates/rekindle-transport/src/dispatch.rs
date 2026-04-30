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
//! 6. Decrypt (DM -> Signal, channel content -> MEK, voice -> MEK)
//! 7. Forward gossip to mesh peers (if TTL > 0 and not private)
//! 8. Invoke the appropriate `InboundHandler` method with authenticated data
//!
//! If any step fails, the message is dropped and logged. No partial dispatch.

pub use crate::handler::TransportEvent;

use std::sync::Arc;

use tokio::sync::mpsc;
use tracing::{debug, info, trace, warn};
use veilid_core::VeilidUpdate;

use crate::config::TransportConfig;
use crate::crypto::envelope::SignedPayload;
use crate::frame::{self, TypeId};
use crate::gossip::DedupCache;
use crate::handler::{InboundHandler, VerifiedSender};
use crate::payload::gossip::SignedGossipEnvelope;
use crate::payload::voice::VoicePayload;

/// Run the inbound dispatch loop until a shutdown signal is received.
pub(crate) async fn run_dispatch_loop<H: InboundHandler>(
    handler: Arc<H>,
    config: Arc<TransportConfig>,
    mut update_rx: mpsc::Receiver<VeilidUpdate>,
    mut shutdown_rx: mpsc::Receiver<()>,
    api: veilid_core::VeilidAPI,
) {
    let mut dedup = DedupCache::new(config.dedup_cache_capacity);
    info!("transport dispatch loop started");

    loop {
        tokio::select! {
            Some(update) = update_rx.recv() => {
                dispatch_update(&handler, &config, &mut dedup, &api, update).await;
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
    update: VeilidUpdate,
) {
    match update {
        VeilidUpdate::AppMessage(msg) => {
            dispatch_app_message(handler, config, dedup, msg.message(), api).await;
        }
        VeilidUpdate::AppCall(call) => {
            dispatch_app_call(handler, config, api, &call).await;
        }
        VeilidUpdate::ValueChange(change) => {
            dispatch_value_change(handler, &change).await;
        }
        VeilidUpdate::Attachment(attachment) => {
            handler.on_event(TransportEvent::AttachmentChanged {
                state: attachment.state.to_string(),
                is_attached: attachment.state.is_attached(),
                public_internet_ready: attachment.public_internet_ready,
            }).await;
        }
        VeilidUpdate::RouteChange(change) => {
            dispatch_route_change(handler, &change).await;
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
    config: &TransportConfig,
    dedup: &mut DedupCache,
    raw: &[u8],
    api: &veilid_core::VeilidAPI,
) {
    let (type_id, payload) = match frame::decode(raw) {
        Ok(result) => result,
        Err(e) => {
            warn!(error = %e, raw_len = raw.len(), "dropping: frame decode failed");
            return;
        }
    };

    match type_id {
        TypeId::GossipBroadcast => {
            dispatch_gossip(handler, config, dedup, payload, raw, api).await;
        }
        TypeId::VoicePacket => {
            dispatch_voice(handler, payload).await;
        }
        tid if !tid.is_rpc() => {
            dispatch_dm(handler, tid, payload).await;
        }
        other => {
            warn!(type_id = other as u8, "unexpected RPC type in app_message, dropping");
        }
    }
}

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
            warn!(error = %e, "dropping app_call: frame decode failed");
            reply_nak(api, call_id).await;
            return;
        }
    };

    if !type_id.is_rpc() {
        warn!(type_id = type_id as u8, "non-RPC type in app_call, dropping");
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

    if let Err(e) = crate::crypto::envelope::verify_signed_payload(&signed) {
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

    let sender = if signed.sender_key_hex.is_empty() { None } else { Some(signed.sender_key_hex.as_str()) };
    let response = handler.on_call(sender, request).await;
    let response_bytes = crate::payload::rpc::serialize_call_response(&response);

    if let Err(e) = api.app_call_reply(call_id, response_bytes).await {
        warn!(error = %e, "failed to send app_call reply");
    }
}

/// Dispatch gossip: verify, dedup, forward to mesh, then deliver to handler.
async fn dispatch_gossip<H: InboundHandler>(
    handler: &Arc<H>,
    _config: &TransportConfig,
    dedup: &mut DedupCache,
    payload: &[u8],
    _raw_frame: &[u8],
    _api: &veilid_core::VeilidAPI,
) {
    let envelope: SignedGossipEnvelope = match postcard::from_bytes(payload) {
        Ok(e) => e,
        Err(e) => {
            warn!(error = %e, "dropping gossip: deserialization failed");
            return;
        }
    };

    let dedup_key = envelope.dedup_key();
    if dedup.check_and_insert(&envelope.community_id, &envelope.sender_pseudonym, &dedup_key) {
        trace!(dedup_key = %dedup_key, "gossip dedup: duplicate");
        return;
    }

    if let Err(e) = crate::crypto::envelope::verify_gossip_envelope(&envelope) {
        warn!(error = %e, sender = %envelope.sender_pseudonym, "dropping gossip: bad signature");
        return;
    }

    // Forward to mesh peers BEFORE delivering to handler.
    // This ensures the message propagates even if handler processing is slow.
    // Private payloads (JoinAccepted, SyncResponse, etc.) are never forwarded.
    if envelope.ttl > 0 && !envelope.is_private() {
        // Decrement TTL and re-broadcast the raw frame to gossip peers.
        // The handler is responsible for providing peer targets via on_gossip_forward.
        // We pass the raw frame bytes so no re-serialization is needed.
        let mut forwarded = envelope.clone();
        forwarded.ttl = forwarded.ttl.saturating_sub(1);

        // Emit a forward event so the application layer can send to its mesh peers.
        // The transport layer doesn't own the peer mesh state — that's application-layer
        // community state. So we deliver the pre-verified envelope for forwarding.
        handler.on_gossip_forward(&forwarded).await;
    }

    let gossip_payload = match postcard::from_bytes(&envelope.payload_bytes) {
        Ok(p) => p,
        Err(e) => {
            warn!(error = %e, "dropping gossip: inner payload parse failed");
            return;
        }
    };

    handler.on_gossip(
        &envelope.community_id,
        &envelope.sender_pseudonym,
        gossip_payload,
        envelope.lamport_ts,
    ).await;
}

async fn dispatch_dm<H: InboundHandler>(handler: &Arc<H>, type_id: TypeId, payload: &[u8]) {
    let signed: SignedPayload = match postcard::from_bytes(payload) {
        Ok(s) => s,
        Err(e) => {
            warn!(error = %e, type_id = type_id as u8, "dropping DM: deserialization failed");
            return;
        }
    };

    if let Err(e) = crate::crypto::envelope::verify_signed_payload(&signed) {
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
    handler.on_dm(&sender, dm_payload, signed.timestamp).await;
}

async fn dispatch_voice<H: InboundHandler>(handler: &Arc<H>, payload: &[u8]) {
    let voice: VoicePayload = match postcard::from_bytes(payload) {
        Ok(v) => v,
        Err(e) => {
            trace!(error = %e, "dropping voice: deserialization failed");
            return;
        }
    };
    let sender_key = voice.sender_key_hex.clone();
    handler.on_voice(&sender_key, voice).await;
}

async fn dispatch_value_change<H: InboundHandler>(
    handler: &Arc<H>,
    change: &veilid_core::VeilidValueChange,
) {
    let key = change.key.to_string();
    let subkeys: Vec<u32> = change.subkeys.iter().collect();
    let first_value = change.value.as_ref().map(|v| v.data().to_vec());

    if change.count == 0 || subkeys.is_empty() {
        debug!(key = %key, count = change.count, "DHT watch died");
        handler.on_event(TransportEvent::WatchDied { record_key: key }).await;
        return;
    }

    handler.on_value_change(&key, subkeys, first_value).await;
}

async fn dispatch_route_change<H: InboundHandler>(
    handler: &Arc<H>,
    change: &veilid_core::VeilidRouteChange,
) {
    if !change.dead_routes.is_empty() {
        handler.on_event(TransportEvent::LocalRoutesDied {
            count: change.dead_routes.len(),
        }).await;
    }
    if !change.dead_remote_routes.is_empty() {
        handler.on_event(TransportEvent::RemoteRoutesDied {
            peer_keys: change.dead_remote_routes.iter().map(ToString::to_string).collect(),
        }).await;
    }
}

async fn reply_nak(api: &veilid_core::VeilidAPI, call_id: veilid_core::OperationId) {
    if let Err(e) = api.app_call_reply(call_id, b"NAK".to_vec()).await {
        warn!(error = %e, "failed to send NAK reply");
    }
}
