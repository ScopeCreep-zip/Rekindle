//! Inbound message dispatcher — forwards raw bytes to TransportCallback.
//!
//! Receives `VeilidUpdate` events from the node's update channel and
//! performs minimal transport-level processing:
//!
//! 1. Read first byte (TypeId) for routing decisions
//! 2. TypeId 0x0A (GossipBroadcast): BLAKE3 content dedup before forwarding
//! 3. Everything else: forward immediately
//!
//! The callback is installed via `parking_lot::RwLock<Option<Arc<dyn TransportCallback>>>`
//! after ChatService construction. Before the callback is installed, events are
//! buffered (bounded at 4096). When the callback becomes available, the buffer
//! is drained first, then live dispatch resumes. Zero events lost during the
//! construction window.
//!
//! `parking_lot::RwLock` is used instead of `ArcSwapOption` because
//! `ArcSwapOption<dyn Trait>` requires `Arc<dyn Trait>: RefCnt` which requires
//! `dyn Trait: Sized` — unsatisfiable for trait objects. The RwLock read is
//! ~2ns uncontended; the write happens exactly once (during `set_callback`).

use std::collections::{HashSet, VecDeque};
use std::sync::Arc;
use std::time::Instant;

use parking_lot::RwLock as CallbackLock;
use tokio::sync::mpsc;
use tracing::{debug, info, trace, warn};
use veilid_core::VeilidUpdate;

use crate::config::TransportConfig;
use crate::shared::{AttachmentState, SharedState};
use rekindle_types::transport::{TransportCallback, TransportEvent};

/// TypeId byte for gossip broadcasts — transport applies BLAKE3 dedup.
const TYPEID_GOSSIP_DEDUP: u8 = 0x0A;

/// Maximum events to buffer while waiting for callback installation.
const EVENT_BUFFER_CAPACITY: usize = 4096;

/// Run the inbound dispatch loop until a shutdown signal is received.
///
/// The callback starts as None. Events are buffered until `set_callback()`
/// is called on the TransportNode, which stores the callback in the RwLock.
/// The loop checks on every iteration — when the callback appears, the
/// buffer is drained first, then live dispatch resumes.
pub(crate) async fn run_dispatch_loop(
    callback: Arc<CallbackLock<Option<Arc<dyn TransportCallback>>>>,
    config: Arc<TransportConfig>,
    mut update_rx: mpsc::Receiver<VeilidUpdate>,
    mut shutdown_rx: mpsc::Receiver<()>,
    api: veilid_core::VeilidAPI,
    shared: Arc<SharedState>,
) {
    let mut gossip_dedup = GossipDedup::new(10_000, 300);
    let mut buffer: Vec<VeilidUpdate> = Vec::new();
    let mut buffer_drained = false;
    // Cache the callback Arc once it's installed to avoid repeated RwLock reads.
    let mut cached_handler: Option<Arc<dyn TransportCallback>> = None;
    info!("transport dispatch loop started — awaiting callback installation");

    loop {
        tokio::select! {
            Some(update) = update_rx.recv() => {
                // Try cached handler first (zero-cost after first installation).
                // If not cached, read the RwLock once, clone the Arc, cache it.
                if cached_handler.is_none() {
                    let guard = callback.read();
                    if let Some(ref cb) = *guard {
                        cached_handler = Some(Arc::clone(cb));
                    }
                    // Guard dropped here — never held across .await
                }

                match cached_handler.as_ref() {
                    Some(handler) => {
                        // Callback installed — drain buffer on first live event
                        if !buffer_drained && !buffer.is_empty() {
                            info!(
                                buffered = buffer.len(),
                                "draining event buffer after callback installation"
                            );
                            for buffered in buffer.drain(..) {
                                dispatch_update(
                                    handler.as_ref(), &config, &mut gossip_dedup,
                                    &api, &shared, buffered,
                                ).await;
                            }
                            buffer_drained = true;
                        }

                        // Process live event
                        dispatch_update(
                            handler.as_ref(), &config, &mut gossip_dedup,
                            &api, &shared, update,
                        ).await;
                    }
                    None => {
                        // No callback yet — buffer the event
                        if buffer.len() < EVENT_BUFFER_CAPACITY {
                            buffer.push(update);
                        } else {
                            let label = veilid_update_label(&update);
                            warn!(
                                label,
                                buffer_size = EVENT_BUFFER_CAPACITY,
                                "event buffer full before callback installation — \
                                 dropping oldest event. This indicates slow ChatService \
                                 construction."
                            );
                            let _ = buffer.remove(0);
                            buffer.push(update);
                        }
                    }
                }
            }
            _ = shutdown_rx.recv() => {
                info!("transport dispatch loop shutting down");
                if !buffer.is_empty() {
                    warn!(
                        dropped = buffer.len(),
                        "shutdown with {} buffered events — events lost",
                        buffer.len()
                    );
                }
                break;
            }
        }
    }
}

async fn dispatch_update(
    handler: &dyn TransportCallback,
    _config: &TransportConfig,
    gossip_dedup: &mut GossipDedup,
    api: &veilid_core::VeilidAPI,
    shared: &SharedState,
    update: VeilidUpdate,
) {
    match update {
        VeilidUpdate::AppMessage(msg) => {
            let sender_key = msg.sender()
                .map(std::string::ToString::to_string)
                .unwrap_or_default();
            let data = msg.message();

            if data.is_empty() {
                debug!("dropping empty app_message");
                return;
            }

            let first_byte = data[0];

            if first_byte == TYPEID_GOSSIP_DEDUP {
                let hash = blake3::hash(&data[1..]);
                if !gossip_dedup.check(hash.as_bytes()) {
                    trace!("gossip dedup: duplicate suppressed");
                    return;
                }
            }

            handler.on_message(&sender_key, data).await;
        }

        VeilidUpdate::AppCall(call) => {
            let sender_key = call.sender()
                .map(std::string::ToString::to_string)
                .unwrap_or_default();
            let data = call.message();
            let call_id = call.id();

            let response = handler.on_call(&sender_key, data).await;

            if let Err(e) = api.app_call_reply(call_id, response).await {
                warn!(error = %e, "app_call_reply failed — caller will timeout");
            }
        }

        VeilidUpdate::ValueChange(change) => {
            let key = change.key.to_string();
            let subkeys: Vec<u32> = change.subkeys.iter().collect();
            let first_value = change.value.as_ref().map(|v| v.data().to_vec());

            if change.count == 0 || subkeys.is_empty() {
                handler.on_event(TransportEvent::WatchExpired {
                    record_key: key,
                }).await;
                return;
            }

            handler.on_record_change(&key, subkeys, change.count, first_value).await;
        }

        VeilidUpdate::Attachment(attachment) => {
            let attached = attachment.state.is_attached();
            let pir = attachment.public_internet_ready;
            let state_str = attachment.state.to_string();
            let att_state = AttachmentState::from_veilid_string(&state_str);
            shared.set_attachment(att_state, attached, pir);

            if attached {
                handler.on_event(TransportEvent::Attached).await;
            } else {
                handler.on_event(TransportEvent::Detached).await;
            }
        }

        VeilidUpdate::RouteChange(change) => {
            for dead in &change.dead_routes {
                handler.on_event(TransportEvent::RouteDied {
                    route_id: dead.to_string(),
                }).await;
            }
            for dead_remote in &change.dead_remote_routes {
                handler.on_event(TransportEvent::RouteDied {
                    route_id: dead_remote.to_string(),
                }).await;
            }
        }

        VeilidUpdate::Shutdown => {
            info!("veilid shutdown event received");
        }

        _ => {
            trace!("ignoring unhandled VeilidUpdate variant");
        }
    }
}

fn veilid_update_label(update: &VeilidUpdate) -> &'static str {
    match update {
        VeilidUpdate::AppCall(_) => "AppCall",
        VeilidUpdate::AppMessage(_) => "AppMessage",
        VeilidUpdate::RouteChange(_) => "RouteChange",
        VeilidUpdate::Attachment(_) => "Attachment",
        VeilidUpdate::ValueChange(_) => "ValueChange",
        VeilidUpdate::Shutdown => "Shutdown",
        _ => "Other",
    }
}

// ── Transport-level gossip dedup ────────────────────────────────────

struct GossipDedup {
    digests: HashSet<[u8; 32]>,
    order: VecDeque<([u8; 32], Instant)>,
    capacity: usize,
    ttl_secs: u64,
}

impl GossipDedup {
    fn new(capacity: usize, ttl_secs: u64) -> Self {
        Self {
            digests: HashSet::with_capacity(capacity),
            order: VecDeque::with_capacity(capacity),
            capacity,
            ttl_secs,
        }
    }

    fn check(&mut self, hash: &[u8; 32]) -> bool {
        let now = Instant::now();
        while let Some((oldest_hash, inserted)) = self.order.front() {
            if now.duration_since(*inserted) > std::time::Duration::from_secs(self.ttl_secs) {
                let h = *oldest_hash;
                self.order.pop_front();
                self.digests.remove(&h);
            } else {
                break;
            }
        }

        if self.digests.contains(hash) {
            return false;
        }

        if self.digests.len() >= self.capacity {
            if let Some((evicted, _)) = self.order.pop_front() {
                self.digests.remove(&evicted);
            }
        }

        self.digests.insert(*hash);
        self.order.push_back((*hash, now));
        true
    }
}
