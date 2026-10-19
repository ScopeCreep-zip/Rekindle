//! Inbound message dispatcher — the single entry point for all network data.
//!
//! Receives the node's [`PortEvent`]s and performs deterministic routing:
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

use rekindle_records::port::{AppCallId, AppMessagePort, PeerTarget, PortEvent, RoutePort};
use tokio::sync::mpsc;
use tracing::{info, warn};

use crate::config::TransportConfig;
use crate::crypto::envelope::{Addressing, SignedPayload};
use crate::frame;
use crate::gossip::DedupCache;
use crate::handler::InboundHandler;
use crate::shared::{AttachmentState, SharedState};

/// The route owners the dispatch loop reports `RouteChange` to.
pub(crate) struct RouteOwners {
    /// Peers' routes and our own.
    pub routes: Arc<dyn RoutePort>,
    /// The peers' cached route blobs.
    pub peers: Arc<parking_lot::RwLock<crate::broadcast::peer_registry::PeerRegistry>>,
}

/// Run the inbound dispatch loop until a shutdown signal is received.
pub(crate) async fn run_dispatch_loop<H: InboundHandler>(
    handler: Arc<H>,
    config: Arc<TransportConfig>,
    mut events: mpsc::Receiver<PortEvent>,
    mut shutdown_rx: mpsc::Receiver<()>,
    messages: Arc<dyn AppMessagePort>,
    shared: Arc<SharedState>,
    routes: RouteOwners,
) {
    let mut dedup = DedupCache::new(config.dedup_cache_capacity);
    info!("transport dispatch loop started");

    loop {
        tokio::select! {
            Some(event) = events.recv() => {
                dispatch_event(&handler, &mut dedup, messages.as_ref(), &shared, &routes, event).await;
            }
            _ = shutdown_rx.recv() => {
                info!("transport dispatch loop shutting down");
                break;
            }
        }
    }
}

async fn dispatch_event<H: InboundHandler>(
    handler: &Arc<H>,
    dedup: &mut DedupCache,
    messages: &dyn AppMessagePort,
    shared: &SharedState,
    routes: &RouteOwners,
    event: PortEvent,
) {
    match event {
        PortEvent::AppMessage { message } => {
            dispatch_app_message(handler, dedup, &message).await;
        }
        PortEvent::AppCall { call_id, message } => {
            dispatch_app_call(handler, messages, call_id, &message).await;
        }
        PortEvent::ValueChange(change) => {
            dispatch_value_change(handler, shared, change).await;
        }
        PortEvent::Attachment {
            state,
            is_attached,
            public_internet_ready,
            reliable_peers,
            live_peers,
            network_size,
            median_latency_us,
        } => {
            let att_state = AttachmentState::from_veilid_string(&state);
            shared.set_attachment(att_state, is_attached, public_internet_ready);
            // 0.5.7's richer attachment signal — recorded so readiness
            // checks and status surfaces can use real peer counts instead
            // of inferring health from the attachment enum alone.
            shared.set_network_health(reliable_peers, live_peers, network_size, median_latency_us);
            handler
                .on_event(TransportEvent::AttachmentChanged {
                    state,
                    is_attached,
                    public_internet_ready,
                })
                .await;
        }
        PortEvent::RouteChange { dead, dead_remote } => {
            dispatch_route_change(handler, routes, &dead, dead_remote).await;
        }
        PortEvent::Shutdown => {
            info!("network shutdown event received");
        }
    }
}

async fn dispatch_app_message<H: InboundHandler>(
    handler: &Arc<H>,
    dedup: &mut DedupCache,
    raw: &[u8],
) {
    // A 1:1 message carries the `'D'` tag; frames start with their
    // version (`0x01`) and gossip with a Cap'n Proto header, so the first
    // byte decides without a trial decode (plan E2.1).
    if raw.first() == Some(&crate::operations::dm::DIRECT_TAG) {
        handler.on_direct(raw).await;
        return;
    }
    let (type_id, _payload) = match frame::decode(raw) {
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

    // No framed type travels as an app message: 1:1 messages are `'D'`
    // datagrams and gossip is the unframed `SignedEnvelope` above.
    warn!(
        type_id = type_id as u8,
        "unexpected frame type in app_message, dropping"
    );
}

const APP_CALL_HANDLER_DEADLINE: std::time::Duration = std::time::Duration::from_secs(12);

async fn dispatch_app_call<H: InboundHandler>(
    handler: &Arc<H>,
    messages: &dyn AppMessagePort,
    call_id: AppCallId,
    raw: &[u8],
) {
    if raw.first() == Some(&crate::operations::dm::DIRECT_TAG) {
        let reply = match tokio::time::timeout(
            APP_CALL_HANDLER_DEADLINE,
            handler.on_direct_call(raw),
        )
        .await
        {
            Ok(Some(reply)) => reply,
            Ok(None) => b"NAK".to_vec(),
            Err(_) => {
                warn!("1:1 app_call handler exceeded deadline — sending NAK");
                b"NAK".to_vec()
            }
        };
        if let Err(e) = messages.reply(call_id, reply).await {
            warn!(error = %e, "failed to send 1:1 app_call reply");
        }
        return;
    }
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
                dispatch_bare_mek_transfer(handler, messages, call_id, transfer).await;
                return;
            }
            warn!(error = %e, "dropping app_call: frame decode failed");
            reply_nak(messages, call_id).await;
            return;
        }
    };

    if !type_id.is_rpc() {
        warn!(
            type_id = type_id as u8,
            "non-RPC type in app_call, dropping"
        );
        reply_nak(messages, call_id).await;
        return;
    }

    let signed: SignedPayload = match postcard::from_bytes(payload) {
        Ok(s) => s,
        Err(e) => {
            warn!(error = %e, "dropping app_call: deserialization failed");
            reply_nak(messages, call_id).await;
            return;
        }
    };

    let Some(me) = handler.local_identity() else {
        warn!("dropping app_call: no unlocked identity");
        reply_nak(messages, call_id).await;
        return;
    };
    let to_me = Addressing {
        recipient: &me,
        type_id,
    };
    if let Err(e) = crate::crypto::envelope::verify_signed_payload(&signed, to_me) {
        warn!(error = %e, sender = %signed.sender_key_hex, "dropping app_call: bad signature");
        reply_nak(messages, call_id).await;
        return;
    }

    let request = match crate::payload::rpc::deserialize_inbound_call(type_id, &signed.payload) {
        Ok(r) => r,
        Err(e) => {
            warn!(error = %e, "dropping app_call: payload parse failed");
            reply_nak(messages, call_id).await;
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

    if let Err(e) = messages.reply(call_id, response_bytes).await {
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
    messages: &dyn AppMessagePort,
    call_id: AppCallId,
    transfer: rekindle_codec::community::envelope::MekTransferPayload,
) {
    use rekindle_codec::community::envelope::{
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
            rekindle_codec::capnp_envelope::encode_community_envelope(&ack).unwrap_or_else(|e| {
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

    if let Err(e) = messages.reply(call_id, response_bytes).await {
        warn!(error = %e, "failed to send MEK transfer reply");
    }
}

async fn dispatch_value_change<H: InboundHandler>(
    handler: &Arc<H>,
    shared: &SharedState,
    change: rekindle_records::port::ValueChange,
) {
    // The pool is the one owner of watch death (plan C7.8): it re-arms the
    // watch of a record it holds, and a record it does not hold was
    // released, so its watch ends by design.
    if let Some(pool) = shared.records() {
        pool.on_value_change(&change);
    }

    // A watch's last notification (`count == 0`) can still carry changes.
    if !change.subkeys.is_empty() {
        let first_value = change.value.map(|v| v.data);
        handler
            .on_value_change(&change.key, change.subkeys, first_value)
            .await;
    }
}

async fn dispatch_route_change<H: InboundHandler>(
    handler: &Arc<H>,
    routes: &RouteOwners,
    dead: &[String],
    dead_remote: Vec<String>,
) {
    if !dead.is_empty() {
        // Ours among the dead are forgotten (never released: the network
        // already dropped them) and reallocated by their owner, off this
        // loop.
        routes.routes.on_dead_own(dead);
        let count = dead.len();
        handler
            .on_event(TransportEvent::LocalRoutesDied { count })
            .await;
    }
    if !dead_remote.is_empty() {
        // The importer forgets them (the network already released them) and
        // the peers holding one of those blobs lose it, so their next send
        // fetches a fresh route (plan C7.9e).
        let dead_remote: Vec<PeerTarget> = dead_remote.into_iter().map(PeerTarget::new).collect();
        let blobs = routes.routes.on_dead_remote(&dead_remote);
        let peer_keys = routes.peers.write().invalidate_blobs(&blobs);
        handler
            .on_event(TransportEvent::RemoteRoutesDied { peer_keys })
            .await;
    }
}

async fn reply_nak(messages: &dyn AppMessagePort, call_id: AppCallId) {
    if let Err(e) = messages.reply(call_id, b"NAK".to_vec()).await {
        warn!(error = %e, "failed to send NAK reply");
    }
}
