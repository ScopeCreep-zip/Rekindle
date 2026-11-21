//! Inbound message dispatcher — the single entry point for all network data.
//!
//! Receives the node's [`PortEvent`]s and performs deterministic routing:
//!
//! 0. Route by the ingress tag (§6, `rekindle_codec::ingress`): `'D'` is a
//!    1:1 message, `'G'` gossip; an untagged `app_message` is dropped, and
//!    an untagged `app_call` is a transport frame (below) or the bare MEK
//!    transfer until E3.5 tags it
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

use rekindle_codec::ingress::Ingress;
use rekindle_records::port::{AppCallId, AppMessagePort, PeerTarget, PortEvent, RoutePort};
use tokio::sync::mpsc;
use tracing::{info, warn};

use crate::config::TransportConfig;
use crate::crypto::envelope::{Addressing, SignedPayload};
use crate::frame;
use crate::gossip::DedupCache;
use crate::handler::InboundHandler;
use crate::liveness::ProbeAnswer;
use crate::operations::direct::{CALL_DEADLINE, NAK};
use crate::shared::{AttachmentState, SharedState};
use crate::subscriptions::value_changes::ValueChanges;

/// The route owners the dispatch loop reports `RouteChange` to.
pub(crate) struct RouteOwners {
    /// Peers' routes and our own.
    pub routes: Arc<dyn RoutePort>,
    /// The peers' cached route blobs.
    pub peers: Arc<parking_lot::RwLock<crate::broadcast::peer_registry::PeerRegistry>>,
    /// Route liveness: what arrives over our routes, and our probes.
    pub liveness: Arc<crate::liveness::Liveness>,
}

/// Run the inbound dispatch loop until a shutdown signal is received, then
/// receive without dispatching through the network's `Shutdown`
/// ([`super::drain`]).
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
    let (value_changes, worker) = ValueChanges::new(Arc::clone(&handler));
    info!("transport dispatch loop started");

    let dispatch = async {
        loop {
            // `biased` (here and below): the branches in this order on
            // every poll, never tokio's per-runtime random draw, so one
            // seed gives one order of events in the netsim (ADR 0021).
            tokio::select! {
                biased;
                _ = shutdown_rx.recv() => {
                    info!("transport dispatch loop shutting down");
                    break;
                }
                Some(event) = events.recv() => {
                    let ctx = DispatchCtx {
                        messages: messages.as_ref(),
                        shared: &shared,
                        routes: &routes,
                        value_changes: &value_changes,
                    };
                    dispatch_event(&handler, &mut dedup, &ctx, event).await;
                }
            }
        }
        // Stopping: dispatch nothing more, but keep the receiver until
        // Veilid's last update, `Shutdown` (live R3-6); the node's
        // `shutdown` signals again once the network stopped.
        super::drain::drain_through_shutdown(&mut events, &mut shutdown_rx, &shared).await;
    };
    // The value changes' worker runs beside the loop in this task and ends
    // with it: at shutdown a change still reading the network is dropped
    // with the node.
    tokio::select! {
        biased;
        () = dispatch => {}
        () = worker => {}
    }
}

/// What the loop lends each event's dispatch.
struct DispatchCtx<'a> {
    messages: &'a dyn AppMessagePort,
    shared: &'a SharedState,
    routes: &'a RouteOwners,
    /// The value changes' own worker: their handlers read the DHT, so
    /// they never run in this loop.
    value_changes: &'a ValueChanges,
}

async fn dispatch_event<H: InboundHandler>(
    handler: &Arc<H>,
    dedup: &mut DedupCache,
    ctx: &DispatchCtx<'_>,
    event: PortEvent,
) {
    let DispatchCtx {
        messages,
        shared,
        routes,
        value_changes,
    } = *ctx;
    match event {
        PortEvent::AppMessage { message, route } => {
            // Arrival over our route proves its inbound path (RH1r-c O1):
            // Veilid names the route that decrypted it.
            routes.liveness.heard_over(route.as_deref());
            dispatch_app_message(handler, dedup, &message).await;
        }
        PortEvent::AppCall {
            call_id,
            message,
            route,
        } => {
            match routes.liveness.answer_probe(&message) {
                ProbeAnswer::NotProbe => {}
                // A liveness probe: ours echoes; another's is refused.
                ProbeAnswer::Echo(echo) => {
                    if let Err(e) = messages.reply(call_id, echo).await {
                        warn!(error = %e, "failed to answer our own route probe");
                    }
                    return;
                }
                // Another's probe: a peer's suspect round, its report that
                // our route failed it (plan LF6): test the route now.
                ProbeAnswer::Refuse => {
                    routes.liveness.route_reported();
                    reply_nak(messages, call_id).await;
                    return;
                }
            }
            routes.liveness.heard_over(route.as_deref());
            dispatch_app_call(handler, messages, call_id, &message).await;
        }
        PortEvent::ValueChange(change) => {
            dispatch_value_change(shared, value_changes, change);
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
            // A reattach, or the public internet ready again, tests our
            // routes at once (RH1r-c O5).
            let regained = (is_attached && !shared.is_attached())
                || (public_internet_ready && !shared.public_internet_ready());
            let changed = shared.set_attachment(att_state, is_attached, public_internet_ready);
            if regained {
                routes.liveness.network_changed();
            }
            // 0.5.7's richer attachment signal — recorded so readiness
            // checks and status surfaces can use real peer counts instead
            // of inferring health from the attachment enum alone.
            shared.set_network_health(reliable_peers, live_peers, network_size, median_latency_us);
            // Veilid 0.5.8 re-sends an unchanged attachment after a stall
            // (`attachment_manager/mod.rs:557-581`): the handler (friends'
            // trigger, the frontends' network event) hears edges only.
            if !changed {
                return;
            }
            handler
                .on_event(TransportEvent::AttachmentChanged {
                    state,
                    is_attached,
                    public_internet_ready,
                })
                .await;
        }
        PortEvent::RouteChange { dead, dead_remote } => {
            dispatch_route_change(handler, routes, &dead, &dead_remote).await;
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
    // Routed by the ingress tag (§6 ledger), so no path trial-decodes: a
    // 1:1 message carries `'D'` and gossip `'G'`. No transport frame
    // travels as an app message, so anything else is dropped (plan E2.3).
    match rekindle_codec::ingress::classify(raw) {
        Ingress::Direct(_) => handler.on_direct(raw).await,
        Ingress::Gossip(envelope) => {
            super::dispatch_gossip::dispatch_bare_gossip(handler, dedup, envelope).await;
        }
        Ingress::Untagged => {
            warn!(raw_len = raw.len(), "dropping untagged app_message");
        }
    }
}

async fn dispatch_app_call<H: InboundHandler>(
    handler: &Arc<H>,
    messages: &dyn AppMessagePort,
    call_id: AppCallId,
    raw: &[u8],
) {
    let ingress = rekindle_codec::ingress::classify(raw);
    if let Ingress::Gossip(_) = ingress {
        // Gossip travels by `app_message`; a `'G'` call is refused before
        // any frame decode (plan audit G10).
        warn!("dropping app_call: gossip is not called");
        reply_nak(messages, call_id).await;
        return;
    }
    if let Ingress::Direct(_) = ingress {
        // The handler answers within the deadline itself
        // (`operations::direct::answer_call`); a call it leaves unanswered
        // gets no reply, so the caller's `app_call` times out and is
        // attempted again (a NAK would be a refusal, RV2).
        let Some(reply) = handler.on_direct_call(raw).await else {
            return;
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
            if let Some(request) = super::bare_envelope::decode_bare_bootstrap_request(raw) {
                dispatch_bare_request(handler, messages, call_id, request).await;
                return;
            }
            if let Some(request) = super::bare_envelope::decode_bare_checkpoint_state_request(raw) {
                dispatch_bare_request(handler, messages, call_id, request).await;
                return;
            }
            if let Some(request) = super::bare_envelope::decode_bare_checkpoint_hint_request(raw) {
                dispatch_bare_request(handler, messages, call_id, request).await;
                return;
            }
            if let Some((attachment_id, requested_chunks)) =
                super::bare_envelope::decode_bare_attachment_request(raw)
            {
                let request = crate::payload::rpc::InboundCall::RequestAttachment {
                    attachment_id,
                    requested_chunks,
                };
                dispatch_bare_attachment_request(handler, messages, call_id, request).await;
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
    if let Err(e) =
        crate::crypto::envelope::verify_signed_payload(&signed, to_me, handler.clock().now_ms())
    {
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

    let response_bytes = if let Ok(response) =
        tokio::time::timeout(CALL_DEADLINE, handler.on_call(sender, request)).await
    {
        crate::payload::rpc::serialize_call_response(&response)
    } else {
        warn!(
            type_id = type_id as u8,
            deadline_secs = CALL_DEADLINE.as_secs(),
            "app_call handler exceeded deadline — sending NAK"
        );
        NAK.to_vec()
    };

    if let Err(e) = messages.reply(call_id, response_bytes).await {
        warn!(error = %e, "failed to send app_call reply");
    }
}

/// Hand a bare MEK transfer to the handler and reply in the same
/// dialect it arrived in.
///
/// The reply is the Cap'n Proto `MekTransferAck` the handler built (only
/// the handler holds the unwrapped key that keys its confirmation tag,
/// review S7m part 1), not a serialized `CallResponse`, because the sender
/// awaits exactly that ack and settles the transfer only on a tag that
/// verifies.
async fn dispatch_bare_mek_transfer<H: InboundHandler>(
    handler: &Arc<H>,
    messages: &dyn AppMessagePort,
    call_id: AppCallId,
    transfer: rekindle_codec::community::envelope::MekTransferPayload,
) {
    let community_id = transfer.community_id.clone();
    // The sender is authenticated by the ECDH wrap, not by an envelope
    // signature, so it is passed through as the claimed sender and the
    // handler's unwrap is what actually decides.
    let sender = transfer.sender_pseudonym.clone();

    let response_bytes = match tokio::time::timeout(
        CALL_DEADLINE,
        handler.on_call(
            Some(sender.as_str()),
            crate::payload::rpc::InboundCall::CommunityMekTransfer(transfer),
        ),
    )
    .await
    {
        // `Ok(bytes)` is the encoded, tagged `MekTransferAck`.
        Ok(crate::payload::rpc::CallResponse::Ok(ack)) => ack,
        Ok(other) => {
            warn!(
                community = %community_id,
                response = ?other,
                "MEK transfer not accepted"
            );
            NAK.to_vec()
        }
        Err(_) => {
            warn!(
                community = %community_id,
                deadline_secs = CALL_DEADLINE.as_secs(),
                "MEK transfer handler exceeded deadline — sending NAK"
            );
            NAK.to_vec()
        }
    };

    if let Err(e) = messages.reply(call_id, response_bytes).await {
        warn!(error = %e, "failed to send MEK transfer reply");
    }
}

/// The reply a requester of attachment chunks reads when we hold none of
/// them: the desktop track's answer, so both shells read alike.
const NOTHING_HELD: &[u8] = b"ACK";

/// Hand a bare attachment request to the handler and reply in its
/// dialect: the encoded `MultiAttachmentChunk` envelope the handler
/// built, [`NOTHING_HELD`] when it holds none of the chunks, and a NAK
/// when it refused or ran past the deadline.
async fn dispatch_bare_attachment_request<H: InboundHandler>(
    handler: &Arc<H>,
    messages: &dyn AppMessagePort,
    call_id: AppCallId,
    request: crate::payload::rpc::InboundCall,
) {
    use crate::payload::rpc::CallResponse;
    let reply = match tokio::time::timeout(CALL_DEADLINE, handler.on_call(None, request)).await {
        Ok(CallResponse::Ok(chunks)) => chunks,
        Ok(CallResponse::Ack) => NOTHING_HELD.to_vec(),
        Ok(CallResponse::Rejected { reason }) => {
            warn!(%reason, "attachment request refused");
            NAK.to_vec()
        }
        Err(_) => {
            warn!(
                deadline_secs = CALL_DEADLINE.as_secs(),
                "attachment request exceeded deadline — sending NAK"
            );
            NAK.to_vec()
        }
    };
    if let Err(e) = messages.reply(call_id, reply).await {
        warn!(error = %e, "failed to send attachment reply");
    }
}

fn dispatch_value_change(
    shared: &SharedState,
    value_changes: &ValueChanges,
    change: rekindle_records::port::ValueChange,
) {
    // The pool is the one owner of watch death (plan C7.8): it re-arms the
    // watch of a record it holds, and a record it does not hold was
    // released, so its watch ends by design.
    if let Some(pool) = shared.records() {
        pool.on_value_change(&change);
    }

    // A watch's last notification (`count == 0`) can still carry changes.
    // The handler reads the network, so it runs on the value changes' own
    // worker, never ahead of an app message in this loop.
    if !change.subkeys.is_empty() {
        value_changes.queue(change);
    }
}

/// Veilid declared these imported routes dead: the importer forgets them
/// (the network already released them) and the peers holding one of those
/// blobs lose it, so their next send fetches a fresh route (plan C7.9e).
/// Returns those peers. Both hosts take a `RouteChange` through this.
pub(crate) fn forget_dead_remote(
    routes: &dyn RoutePort,
    peers: &parking_lot::RwLock<crate::broadcast::peer_registry::PeerRegistry>,
    dead: &[String],
) -> Vec<String> {
    let dead: Vec<PeerTarget> = dead.iter().cloned().map(PeerTarget::new).collect();
    let blobs = routes.on_dead_remote(&dead);
    if blobs.is_empty() {
        return Vec::new();
    }
    peers.write().invalidate_blobs(&blobs)
}

async fn dispatch_route_change<H: InboundHandler>(
    handler: &Arc<H>,
    routes: &RouteOwners,
    dead: &[String],
    dead_remote: &[String],
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
        let peer_keys = forget_dead_remote(routes.routes.as_ref(), &routes.peers, dead_remote);
        handler
            .on_event(TransportEvent::RemoteRoutesDied { peer_keys })
            .await;
    }
}

async fn reply_nak(messages: &dyn AppMessagePort, call_id: AppCallId) {
    if let Err(e) = messages.reply(call_id, NAK.to_vec()).await {
        warn!(error = %e, "failed to send NAK reply");
    }
}

#[cfg(test)]
#[path = "dispatch_tests.rs"]
mod tests;

/// Answer a joiner's bare `BootstrapRequest` (plan E3.2) with the
/// handler's encoded `bootstrapResponseV2`, or a member's bare
/// `CheckpointStateRequest` (S7c.3) with its `checkpointStateResponse`; a
/// refusal, or a handler past the deadline, is a NAK.
async fn dispatch_bare_request<H: InboundHandler>(
    handler: &Arc<H>,
    messages: &dyn AppMessagePort,
    call_id: AppCallId,
    request: crate::payload::rpc::InboundCall,
) {
    let reply = match tokio::time::timeout(CALL_DEADLINE, handler.on_call(None, request)).await {
        Ok(crate::payload::rpc::CallResponse::Ok(bytes)) => bytes,
        Ok(other) => {
            warn!(response = ?other, "bare request refused");
            NAK.to_vec()
        }
        Err(_) => {
            warn!("bare request exceeded the call deadline");
            NAK.to_vec()
        }
    };
    if let Err(e) = messages.reply(call_id, reply).await {
        warn!(error = %e, "failed to send bare request reply");
    }
}
