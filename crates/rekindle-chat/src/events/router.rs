//! Inbound event reader — reads InboundEvent from mpsc channel,
//! verifies inbound messages, dispatches to services, emits events
//! through the pipeline.
//!
//! Verification order for Message events:
//! 1. Read TypeId byte (first byte of raw data)
//! 2. For DM TypeIds (0x01-0x06): parse SignedEnvelope, verify Ed25519
//!    signature + timestamp freshness (5min window, 60s future skew)
//! 3. For gossip TypeId (0x0A): delegate to community.handle_gossip
//! 4. For RPC TypeId (0x0B): delegate to community.handle_rpc_message
//! 5. Convert verified payload to SubscriptionEvent
//! 6. Process through EventPipeline (state_effects → dedup → emit)

use std::sync::Arc;

use tokio::sync::mpsc;
use rekindle_types::transport::{InboundEvent, TransportEvent};
use rekindle_types::subscription_events::SubscriptionEvent;

use super::pipeline::EventPipeline;
use super::registry::{WatchKind, WatchRegistry};
use crate::crypto::envelope::SignedEnvelope;
use crate::dm::DmDeps;
use crate::friendship::FriendshipService;
use crate::messaging::MessagingService;
use crate::community::CommunityService;

/// Run the inbound event reader loop. Spawned by ChatService after
/// receiving the mpsc::Receiver from TransportNode::start().
///
/// Runs until the channel closes (transport shutdown).
pub async fn run_inbound_loop(
    mut rx: mpsc::Receiver<InboundEvent>,
    watches: Arc<WatchRegistry>,
    pipeline: Arc<EventPipeline>,
    friendship: Arc<FriendshipService>,
    messaging: Arc<MessagingService>,
    community: Arc<CommunityService>,
    dm_deps: Arc<dyn DmDeps>,
) {
    tracing::info!("router: inbound event reader started");

    while let Some(event) = rx.recv().await {
        match event {
            InboundEvent::Message { sender_key, data } => {
                let type_id = data.first().copied().unwrap_or(0);
                tracing::info!(
                    type_id,
                    data_len = data.len(),
                    sender = if sender_key.is_empty() { "anonymous" } else { &sender_key[..16.min(sender_key.len())] },
                    "router: InboundEvent::Message"
                );
                handle_message(
                    &sender_key, &data, &pipeline, &friendship,
                    &messaging, &community, &*dm_deps,
                ).await;
            }
            InboundEvent::Call { sender_key, data, reply_tx } => {
                let response = community.handle_rpc_call(&sender_key, &data, &*dm_deps).await;
                let _ = reply_tx.send(response);
            }
            InboundEvent::RecordChange { record_key, subkeys, count: _, data } => {
                handle_record_change(
                    &record_key, &subkeys, data, &watches, &friendship,
                    &messaging, &community, &*dm_deps,
                ).await;
            }
            InboundEvent::Event(transport_event) => {
                handle_transport_event(transport_event, &pipeline);
            }
            InboundEvent::TransferProgress { transfer_id, total_size, bytes_transferred, status, .. } => {
                pipeline.process(SubscriptionEvent::BulkTransferProgress {
                    transfer_id: hex::encode(transfer_id),
                    stream_id: 0,
                    direction: "receive".into(),
                    bytes_transferred,
                    total_size,
                    status: format!("{status:?}"),
                });
            }
            InboundEvent::TransferOffer { transfer_id, sender_peer_key, filename, total_size, .. } => {
                tracing::info!(
                    filename = %filename,
                    size = total_size,
                    sender = &sender_peer_key[..16.min(sender_peer_key.len())],
                    "router: transfer offer"
                );
                pipeline.process(SubscriptionEvent::BulkTransferProgress {
                    transfer_id: hex::encode(transfer_id),
                    stream_id: 0, direction: "receive".into(),
                    bytes_transferred: 0, total_size, status: "Offering".into(),
                });
            }
            InboundEvent::TransferComplete { transfer_id, path, hash_match } => {
                tracing::info!(path = %path, hash_match, "router: transfer complete");
                pipeline.process(SubscriptionEvent::BulkTransferProgress {
                    transfer_id: hex::encode(transfer_id),
                    stream_id: 0, direction: "receive".into(),
                    bytes_transferred: 0, total_size: 0,
                    status: if hash_match { "Completed".into() } else { "Failed".into() },
                });
            }
            InboundEvent::TransferFailed { transfer_id, reason } => {
                tracing::warn!(reason = %reason, "router: transfer failed");
                pipeline.process(SubscriptionEvent::BulkTransferProgress {
                    transfer_id: hex::encode(transfer_id),
                    stream_id: 0, direction: "receive".into(),
                    bytes_transferred: 0, total_size: 0,
                    status: format!("Failed: {reason}"),
                });
            }
        }
    }

    tracing::info!("router: inbound event reader exiting — channel closed");
}

/// Handle an inbound Message (TypeId dispatch).
async fn handle_message(
    sender_key: &str,
    data: &[u8],
    pipeline: &EventPipeline,
    friendship: &FriendshipService,
    messaging: &MessagingService,
    community: &CommunityService,
    _dm_deps: &dyn DmDeps,
) {
    if data.is_empty() { return; }
    let type_id = data[0];

    match type_id {
        1..=6 => {
            let envelope = match SignedEnvelope::parse(data) {
                Ok(e) => e,
                Err(e) => {
                    tracing::warn!(
                        type_id,
                        sender = &sender_key[..12.min(sender_key.len())],
                        error = %e,
                        "router: DM envelope parse failed"
                    );
                    return;
                }
            };
            if let Err(e) = envelope.verify() {
                tracing::warn!(
                    type_id,
                    sender = &sender_key[..12.min(sender_key.len())],
                    error = %e,
                    "router: DM verification failed"
                );
                return;
            }
            let verified_sender = hex::encode(&envelope.sender_key[..]);
            let payload = &envelope.payload;

            match type_id {
                1 => {
                    if !messaging.handle_typing(&verified_sender, payload) { return; }
                    if let Ok(dm_payload) = postcard::from_bytes::<rekindle_types::dm_payload::DmPayload>(payload) {
                        pipeline.process(dm_payload.into_event(&verified_sender));
                    }
                }
                2 => {
                    friendship.trigger_inbox_scan();
                    pipeline.process(SubscriptionEvent::Friend(
                        rekindle_types::subscription_events::FriendEvent::RequestAcknowledged {
                            peer_key: verified_sender,
                        },
                    ));
                }
                3 => {
                    friendship.handle_unfriend(&verified_sender).await;
                    pipeline.process(SubscriptionEvent::Friend(
                        rekindle_types::subscription_events::FriendEvent::Removed {
                            peer_key: verified_sender,
                        },
                    ));
                }
                4 => {
                    pipeline.process(SubscriptionEvent::Friend(
                        rekindle_types::subscription_events::FriendEvent::RemoveAcknowledged {
                            peer_key: verified_sender,
                        },
                    ));
                }
                5 => {
                    friendship.handle_profile_rotated(&verified_sender, payload).await;
                    if let Ok(dm_payload) = postcard::from_bytes::<rekindle_types::dm_payload::DmPayload>(payload) {
                        pipeline.process(dm_payload.into_event(&verified_sender));
                    }
                }
                6 => {
                    if !messaging.handle_presence_update(&verified_sender, payload) { return; }
                    if let Ok(dm_payload) = postcard::from_bytes::<rekindle_types::dm_payload::DmPayload>(payload) {
                        pipeline.process(dm_payload.into_event(&verified_sender));
                    }
                }
                _ => unreachable!(),
            }
        }
        10 => {
            let payload = &data[1..];
            tracing::info!(
                payload_len = payload.len(),
                "router: TypeId=10 gossip → handle_gossip"
            );
            match community.handle_gossip(sender_key, payload).await {
                Some(event) => {
                    tracing::info!("router: handle_gossip returned event → pipeline.process");
                    pipeline.process(event);
                }
                None => {
                    tracing::warn!("router: handle_gossip returned None — dropped");
                }
            }
        }
        11 => {
            let payload = &data[1..];
            community.handle_rpc_message(sender_key, payload).await;
        }
        _ => {
            tracing::debug!(
                type_id,
                sender = &sender_key[..12.min(sender_key.len())],
                "router: unknown TypeId"
            );
        }
    }
}

/// Handle a DHT record change (watch dispatch).
async fn handle_record_change(
    record_key: &str,
    subkeys: &[u32],
    data: Option<Vec<u8>>,
    watches: &WatchRegistry,
    friendship: &FriendshipService,
    messaging: &MessagingService,
    community_svc: &CommunityService,
    dm_deps: &dyn DmDeps,
) {
    match watches.lookup(record_key) {
        Some(WatchKind::DmLog { ref peer_key }) => {
            messaging.handle_dm_log_change(peer_key, record_key, data).await;
        }
        Some(WatchKind::DmSmpl { record_key: ref rk }) => {
            for &subkey in subkeys {
                if let Err(e) = crate::dm::handle_dm_subkey_change(
                    dm_deps, rk, subkey, data.as_deref(),
                ).await {
                    tracing::warn!(
                        record_key = &rk[..12.min(rk.len())],
                        subkey,
                        error = %e,
                        "router: DmSmpl subkey change failed"
                    );
                }
            }
        }
        Some(WatchKind::ChannelLog { ref community, ref channel_id, ref member }) => {
            messaging.handle_channel_log_change(community, channel_id, member, data);
        }
        Some(WatchKind::FriendInbox) => {
            friendship.trigger_inbox_scan();
        }
        Some(WatchKind::GovernanceManifest { ref community }) => {
            community_svc.handle_governance_change(community, subkeys).await;
        }
        Some(WatchKind::MemberRegistry { ref community }) => {
            community_svc.handle_registry_change(community, subkeys).await;
        }
        Some(WatchKind::JoinInbox { ref community }) => {
            community_svc.handle_join_inbox_change(community).await;
        }
        None => {
            tracing::debug!(record_key, "router: record change for unregistered watch");
        }
    }
}

/// Handle a transport lifecycle event.
fn handle_transport_event(event: TransportEvent, pipeline: &EventPipeline) {
    match event {
        TransportEvent::Attached => {
            tracing::info!("router: transport attached");
            pipeline.process(SubscriptionEvent::Network(
                rekindle_types::subscription_events::NetworkEvent::AttachmentChanged {
                    is_attached: true,
                    public_internet_ready: true,
                },
            ));
        }
        TransportEvent::Detached => {
            tracing::warn!("router: transport detached");
            pipeline.process(SubscriptionEvent::Network(
                rekindle_types::subscription_events::NetworkEvent::AttachmentChanged {
                    is_attached: false,
                    public_internet_ready: false,
                },
            ));
        }
        TransportEvent::RouteDied { ref route_id } => {
            tracing::warn!(route_id, "router: route died");
            pipeline.process(SubscriptionEvent::Network(
                rekindle_types::subscription_events::NetworkEvent::LocalRoutesDied { count: 1 },
            ));
        }
        TransportEvent::WatchExpired { ref record_key } => {
            pipeline.process(SubscriptionEvent::Network(
                rekindle_types::subscription_events::NetworkEvent::WatchFailed {
                    record_key: record_key.clone(),
                    error: "expired".into(),
                },
            ));
        }
        TransportEvent::RouteAllocated { .. } => {}
        TransportEvent::PeerCountChanged { count } => {
            tracing::debug!(count, "router: peer count changed");
        }
        TransportEvent::PublicInternet { available } => {
            tracing::info!(available, "router: public internet changed");
        }
    }
}
