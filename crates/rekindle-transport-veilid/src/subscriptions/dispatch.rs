//! Inbound message dispatcher — sends typed InboundEvent to mpsc channel.
//!
//! Receives `VeilidUpdate` events from the Veilid node's update channel and:
//! 1. TypeId 0x30 (BulkTransfer): handled internally — decode, route to
//!    TransferRegistry, send reply frames via DeliveryEngine. Never forwarded
//!    to the chat layer.
//! 2. TypeId 0x0A (GossipBroadcast): BLAKE3 dedup via OpaqueSeenSet from
//!    rekindle-transport-buff, then forward as InboundEvent::Message.
//! 3. All other AppMessage: forward as InboundEvent::Message.
//! 4. AppCall: forward as InboundEvent::Call with oneshot reply_tx.
//!    Chat layer sends response bytes via reply_tx. Dispatch calls
//!    api.app_call_reply() internally. No Veilid types leak.
//! 5. ValueChange: forward as InboundEvent::RecordChange.
//! 6. Attachment/RouteChange: forward as InboundEvent::Event.
//!
//! No callback RwLock. No lazy installation. No buffering. All deps
//! available at spawn time. The mpsc channel IS the buffer (bounded 4096).

use std::sync::Arc;

use rekindle_transport_buff::OpaqueSeenSet;
use tokio::sync::mpsc;
use tracing::{debug, info, trace, warn};
use veilid_core::VeilidUpdate;

use crate::shared::{AttachmentState, SharedState};
use rekindle_types::transport::{InboundEvent, TransportEvent};

const TYPEID_GOSSIP_DEDUP: u8 = 0x0A;

/// Run the inbound dispatch loop. All deps passed at spawn time.
/// No lazy callback. No buffering. mpsc channel provides backpressure.
pub(crate) async fn run_dispatch_loop(
    inbound_tx: mpsc::Sender<InboundEvent>,
    _config: Arc<crate::config::TransportConfig>,
    mut update_rx: mpsc::Receiver<VeilidUpdate>,
    mut shutdown_rx: mpsc::Receiver<()>,
    api: veilid_core::VeilidAPI,
    shared: Arc<SharedState>,
    transfer_registry: Arc<crate::bulk_transfer::TransferRegistry>,
    delivery: Arc<crate::delivery::DeliveryEngine>,
) {
    let gossip_dedup = OpaqueSeenSet::new(16, 10_000);
    info!("transport dispatch loop started — all deps available, no buffering");

    loop {
        tokio::select! {
            Some(update) = update_rx.recv() => {
                dispatch_update(
                    &inbound_tx, &gossip_dedup, &api, &shared,
                    &transfer_registry, &delivery, update,
                ).await;
            }
            _ = shutdown_rx.recv() => {
                info!("transport dispatch loop shutting down");
                break;
            }
        }
    }
}

async fn dispatch_update(
    inbound_tx: &mpsc::Sender<InboundEvent>,
    gossip_dedup: &OpaqueSeenSet,
    api: &veilid_core::VeilidAPI,
    shared: &SharedState,
    transfer_registry: &crate::bulk_transfer::TransferRegistry,
    delivery: &Arc<crate::delivery::DeliveryEngine>,
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
            info!(
                type_id = first_byte,
                data_len = data.len(),
                sender = if sender_key.is_empty() { "anonymous" } else { &sender_key[..16.min(sender_key.len())] },
                "dispatch: AppMessage received"
            );

            // ── Bulk transfer (0x30): handled internally ────────────
            if first_byte == crate::bulk_transfer::TYPEID_BULK_TRANSFER {
                match crate::bulk_transfer::TransferFrame::decode(data) {
                    Ok(frame) => {
                        let tid = *frame.transfer_id();

                        let delivery_clone = delivery.clone();
                        transfer_registry.handle_frame(frame, |peer_key, reply_frame| {
                            let de = delivery_clone.clone();
                            let pk = peer_key.to_string();
                            Box::pin(async move {
                                if let Ok(wire) = reply_frame.encode() {
                                    let _ = de.deliver(&pk, &wire, rekindle_types::transport::Durability::Ephemeral).await;
                                }
                            })
                        }).await;

                        if let Some(progress) = transfer_registry.transfer_progress(&tid) {
                            let _ = inbound_tx.send(InboundEvent::TransferProgress {
                                transfer_id: progress.transfer_id,
                                filename: progress.filename,
                                total_size: progress.total_size,
                                bytes_transferred: progress.bytes_transferred,
                                chunks_received: progress.chunks_received,
                                chunk_count: progress.chunk_count,
                                status: progress.status,
                            }).await;
                        }
                    }
                    Err(e) => {
                        warn!(error = %e, "bulk transfer frame decode failed");
                    }
                }
                return;
            }

            // ── Gossip dedup (0x0A): OpaqueSeenSet from buff ───────
            if first_byte == TYPEID_GOSSIP_DEDUP {
                let hash = *blake3::hash(&data[1..]).as_bytes();
                if !gossip_dedup.insert(hash) {
                    debug!(data_len = data.len(), "dispatch: gossip dedup SUPPRESSED duplicate");
                    return;
                }
                info!(data_len = data.len(), hash = hex::encode(&hash[..8]), "dispatch: gossip dedup PASSED — forwarding to chat");
            }

            // ── Forward to chat layer ───────────────────────────────
            if let Err(e) = inbound_tx.send(InboundEvent::Message {
                sender_key: sender_key.clone(),
                data: data.to_vec(),
            }).await {
                warn!(type_id = first_byte, "dispatch: inbound_tx.send FAILED — chat channel closed: {e}");
                return;
            }
            info!(type_id = first_byte, data_len = data.len(), "dispatch: forwarded to chat layer via inbound_tx");
        }

        VeilidUpdate::AppCall(call) => {
            let sender_key = call.sender()
                .map(std::string::ToString::to_string)
                .unwrap_or_default();
            let data = call.message().to_vec();
            let call_id = call.id();

            let (reply_tx, reply_rx) = tokio::sync::oneshot::channel::<Vec<u8>>();

            let _ = inbound_tx.send(InboundEvent::Call {
                sender_key,
                data,
                reply_tx,
            }).await;

            let api_clone = api.clone();
            tokio::spawn(async move {
                match reply_rx.await {
                    Ok(response) => {
                        if let Err(e) = api_clone.app_call_reply(call_id, response).await {
                            warn!(error = %e, "app_call_reply failed");
                        }
                    }
                    Err(_) => {
                        warn!("app_call reply_tx dropped — caller will timeout");
                    }
                }
            });
        }

        VeilidUpdate::ValueChange(change) => {
            let key = change.key.to_string();
            let subkeys: Vec<u32> = change.subkeys.iter().collect();
            let first_value = change.value.as_ref().map(|v| v.data().to_vec());

            if change.count == 0 || subkeys.is_empty() {
                let _ = inbound_tx.send(InboundEvent::Event(TransportEvent::WatchExpired {
                    record_key: key,
                })).await;
                return;
            }

            let _ = inbound_tx.send(InboundEvent::RecordChange {
                record_key: key,
                subkeys,
                count: change.count,
                data: first_value,
            }).await;
        }

        VeilidUpdate::Attachment(attachment) => {
            let attached = attachment.state.is_attached();
            let pir = attachment.public_internet_ready;
            let state_str = attachment.state.to_string();
            let att_state = AttachmentState::from_veilid_string(&state_str);
            shared.set_attachment(att_state, attached, pir);

            let event = if attached { TransportEvent::Attached } else { TransportEvent::Detached };
            let _ = inbound_tx.send(InboundEvent::Event(event)).await;
        }

        VeilidUpdate::RouteChange(change) => {
            for dead in &change.dead_routes {
                let _ = inbound_tx.send(InboundEvent::Event(TransportEvent::RouteDied {
                    route_id: dead.to_string(),
                })).await;
            }
            for dead_remote in &change.dead_remote_routes {
                let _ = inbound_tx.send(InboundEvent::Event(TransportEvent::RouteDied {
                    route_id: dead_remote.to_string(),
                })).await;
            }
        }

        VeilidUpdate::Shutdown => { info!("veilid shutdown event received"); }
        _ => { trace!("ignoring unhandled VeilidUpdate"); }
    }
}
