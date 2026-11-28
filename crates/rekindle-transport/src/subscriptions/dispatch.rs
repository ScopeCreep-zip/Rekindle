//! Inbound message dispatcher — the single entry point for all network data.
//!
//! Demultiplexes the node's port events into the app-call, gossip and
//! control workers. Media is timestamped and handed off synchronously.

pub use crate::handler::TransportEvent;

use std::sync::Arc;

use rekindle_codec::capnp_envelope::decode_signed_envelope;
use rekindle_codec::ingress::{classify, Ingress};
use rekindle_records::port::{AppCallId, AppMessagePort, PeerTarget, PortEvent, RoutePort};
use tokio::sync::mpsc;
use tokio::task::JoinSet;
use tracing::{info, warn, Instrument};

use crate::config::TransportConfig;
use crate::crypto::envelope::{Addressing, SignedPayload};
use crate::frame;
use crate::gossip::DedupCache;
use crate::handler::InboundHandler;
use crate::liveness::ProbeAnswer;
use crate::operations::direct::{CALL_DEADLINE, NAK};
use crate::shared::{AttachmentState, SharedState};
use crate::subscriptions::bare_envelope;
use crate::subscriptions::dispatch_bare::{
    dispatch_bare_attachment_request, dispatch_bare_media_key, dispatch_bare_mek_transfer,
    dispatch_bare_request,
};
use crate::subscriptions::lanes::{
    AppCallLane, ControlIngressQueue, GossipIngressQueue, IngressItem,
};
use crate::subscriptions::media_key::MediaKeyAdmission;
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
    let (value_changes, value_worker) = ValueChanges::new(Arc::clone(&handler));
    let inbound = Arc::new(InboundLanes {
        calls: AppCallLane::default(),
        messages,
        media_keys: Arc::default(),
        gossip: GossipIngressQueue::new(),
        control: ControlIngressQueue::new(),
    });
    let mut workers = JoinSet::new();
    workers.spawn(value_worker.in_current_span());
    workers.spawn({
        let handler = Arc::clone(&handler);
        let inbound = Arc::clone(&inbound);
        async move { run_gossip(&handler, &inbound.gossip, config.dedup_cache_capacity).await }
            .in_current_span()
    });
    workers.spawn({
        let handler = Arc::clone(&handler);
        let inbound = Arc::clone(&inbound);
        async move { run_control(handler.as_ref(), &inbound).await }.in_current_span()
    });
    info!("transport dispatch loop started");

    loop {
        tokio::select! {
            biased;
            _ = shutdown_rx.recv() => break,
            result = workers.join_next() => {
                warn!(?result, "inbound worker stopped during dispatch");
                break;
            }
            event = events.recv() => {
                let Some(event) = event else { return };
                dispatch_event(&handler, &shared, &routes, &value_changes, &inbound, event);
            }
        }
    }
    workers.abort_all();
    info!("transport dispatch loop shutting down");
    super::drain::drain_through_shutdown(&mut events, &mut shutdown_rx, &shared).await;
}

struct InboundLanes {
    calls: AppCallLane,
    messages: Arc<dyn AppMessagePort>,
    media_keys: Arc<parking_lot::Mutex<MediaKeyAdmission>>,
    gossip: GossipIngressQueue,
    control: ControlIngressQueue<ControlWork>,
}

enum ControlWork {
    Event(TransportEvent),
    Probe(AppCallId),
}

async fn run_gossip<H: InboundHandler>(
    handler: &Arc<H>,
    queue: &GossipIngressQueue,
    dedup_capacity: usize,
) {
    let mut dedup = DedupCache::new(dedup_capacity);
    loop {
        process_gossip(handler, &mut dedup, queue.pop().await).await;
    }
}

async fn process_gossip<H: InboundHandler>(
    handler: &Arc<H>,
    dedup: &mut DedupCache,
    item: IngressItem,
) {
    match item {
        IngressItem::Direct(datagram) => handler.on_direct(&datagram).await,
        IngressItem::Gossip { signed, .. } => {
            super::dispatch_gossip::dispatch_bare_gossip(handler, dedup, signed).await;
        }
    }
}

async fn run_control<H: InboundHandler>(handler: &H, lanes: &InboundLanes) {
    loop {
        match lanes.control.pop().await {
            ControlWork::Event(event) => handler.on_event(event).await,
            ControlWork::Probe(call_id) => reply_nak(lanes.messages.as_ref(), call_id).await,
        }
    }
}

fn dispatch_event<H: InboundHandler>(
    handler: &Arc<H>,
    shared: &SharedState,
    routes: &RouteOwners,
    value_changes: &ValueChanges,
    lanes: &InboundLanes,
    event: PortEvent,
) {
    match event {
        PortEvent::AppMessage { message, route } => {
            let arrived = tokio::time::Instant::now().into_std();
            // Arrival over our route proves its inbound path (RH1r-c O1):
            // Veilid names the route that decrypted it.
            routes.liveness.heard_over(route.as_deref());
            dispatch_app_message(handler.as_ref(), &lanes.gossip, message, arrived);
        }
        PortEvent::AppCall {
            call_id,
            message,
            route,
        } => {
            match crate::liveness::answer_probe(&message) {
                ProbeAnswer::NotProbe => {}
                // A peer's probe (its suspect round, plan LF6): the NAK
                // tells it the call arrived. It names no sender, so it is
                // no evidence about our route (plan RV2).
                ProbeAnswer::Refuse => {
                    lanes.control.push(ControlWork::Probe(call_id));
                    return;
                }
            }
            routes.liveness.heard_over(route.as_deref());
            lane_app_call(handler, lanes, call_id, message, route);
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
            let changed = shared.set_attachment(att_state, is_attached, public_internet_ready);
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
            lanes
                .control
                .push(ControlWork::Event(TransportEvent::AttachmentChanged {
                    state,
                    is_attached,
                    public_internet_ready,
                }));
        }
        PortEvent::RouteChange { dead, dead_remote } => {
            dispatch_route_change(&lanes.control, routes, &dead, &dead_remote);
        }
        PortEvent::Shutdown => {
            info!("network shutdown event received");
        }
    }
}

fn dispatch_app_message<H: InboundHandler>(
    handler: &H,
    gossip: &GossipIngressQueue,
    raw: Vec<u8>,
    arrived: std::time::Instant,
) {
    match classify(&raw) {
        Ingress::Media(datagram) => handler.on_media(datagram, arrived),
        Ingress::Direct(_) => gossip.push(IngressItem::Direct(raw)),
        Ingress::Gossip(envelope) => match decode_signed_envelope(envelope) {
            Ok(signed) => gossip.push(IngressItem::Gossip {
                signed,
                is_video: false,
            }),
            Err(error) => warn!(%error, "dropping malformed gossip envelope"),
        },
        Ingress::Untagged => warn!(raw_len = raw.len(), "dropping untagged app_message"),
    }
}

fn lane_app_call<H: InboundHandler>(
    handler: &Arc<H>,
    lanes: &InboundLanes,
    call_id: AppCallId,
    message: Vec<u8>,
    route: Option<String>,
) {
    let worker = Arc::clone(handler);
    let messages = Arc::clone(&lanes.messages);
    let media_keys = Arc::clone(&lanes.media_keys);
    lanes.calls.dispatch(&handler.scope(), async move {
        dispatch_app_call(
            &worker,
            messages.as_ref(),
            &media_keys,
            call_id,
            &message,
            route.as_deref(),
        )
        .await;
    });
}

async fn dispatch_app_call<H: InboundHandler>(
    handler: &Arc<H>,
    messages: &dyn AppMessagePort,
    media_keys: &parking_lot::Mutex<MediaKeyAdmission>,
    call_id: AppCallId,
    raw: &[u8],
    route: Option<&str>,
) {
    let ingress = classify(raw);
    if matches!(ingress, Ingress::Gossip(_) | Ingress::Media(_)) {
        // Gossip travels by `app_message`; a `'G'` call is refused before
        // any frame decode (plan audit G10).
        warn!("dropping app_call: gossip and media use app_message");
        reply_nak(messages, call_id).await;
        return;
    }
    if let Ingress::Direct(_) = ingress {
        // The handler answers within the deadline itself
        // (`operations::direct::answer_call`); a call it leaves unanswered
        // gets no reply, so the caller's `app_call` times out and is
        // attempted again (a NAK would be a refusal, RV2). The route it
        // arrived over lets a re-sent call count against ours (plan RV2).
        let Some(reply) = handler.on_direct_call(raw, route).await else {
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
            // Unframed calls are restricted by their envelope's control variant.
            if let Some(transfer) = bare_envelope::decode_bare_mek_transfer(raw) {
                dispatch_bare_mek_transfer(handler, messages, call_id, transfer).await;
                return;
            }
            if let Some(payload) = bare_envelope::decode_bare_media_key(raw) {
                let admitted = media_keys
                    .lock()
                    .admit(&payload, tokio::time::Instant::now().into_std());
                if admitted {
                    dispatch_bare_media_key(handler.as_ref(), messages, call_id, &payload).await;
                } else {
                    reply_nak(messages, call_id).await;
                }
                return;
            }
            if let Some(request) = bare_envelope::decode_bare_bootstrap_request(raw) {
                dispatch_bare_request(handler, messages, call_id, request).await;
                return;
            }
            if let Some(request) = bare_envelope::decode_bare_checkpoint_state_request(raw) {
                dispatch_bare_request(handler, messages, call_id, request).await;
                return;
            }
            if let Some(request) = bare_envelope::decode_bare_checkpoint_hint_request(raw) {
                dispatch_bare_request(handler, messages, call_id, request).await;
                return;
            }
            if let Some(request) = bare_envelope::decode_bare_frontier_request(raw) {
                dispatch_bare_request(handler, messages, call_id, request).await;
                return;
            }
            if let Some((attachment_id, requested_chunks)) =
                bare_envelope::decode_bare_attachment_request(raw)
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
/// The blobs are declared dead, so no caller imports one again for the
/// failure cache's span (plan RV1g B1). Returns those peers. Both hosts
/// take a `RouteChange` through this.
pub(crate) fn forget_dead_remote(
    routes: &dyn RoutePort,
    peers: &parking_lot::RwLock<crate::broadcast::peer_registry::PeerRegistry>,
    liveness: &crate::liveness::Liveness,
    dead: &[String],
) -> Vec<String> {
    let dead: Vec<PeerTarget> = dead.iter().cloned().map(PeerTarget::new).collect();
    let blobs = routes.on_dead_remote(&dead);
    if blobs.is_empty() {
        return Vec::new();
    }
    liveness.declare_dead_by_veilid(&blobs);
    peers.write().invalidate_blobs(&blobs)
}

fn dispatch_route_change(
    control: &ControlIngressQueue<ControlWork>,
    routes: &RouteOwners,
    dead: &[String],
    dead_remote: &[String],
) {
    if !dead.is_empty() {
        routes.routes.on_dead_own(dead);
        control.push(ControlWork::Event(TransportEvent::LocalRoutesDied {
            count: dead.len(),
        }));
    }
    if !dead_remote.is_empty() {
        let peer_keys = forget_dead_remote(
            routes.routes.as_ref(),
            &routes.peers,
            &routes.liveness,
            dead_remote,
        );
        control.push(ControlWork::Event(TransportEvent::RemoteRoutesDied {
            peer_keys,
        }));
    }
}

pub(super) async fn reply_nak(messages: &dyn AppMessagePort, call_id: AppCallId) {
    if let Err(e) = messages.reply(call_id, NAK.to_vec()).await {
        warn!(error = %e, "failed to send NAK reply");
    }
}

#[cfg(test)]
#[path = "dispatch_tests.rs"]
mod tests;
