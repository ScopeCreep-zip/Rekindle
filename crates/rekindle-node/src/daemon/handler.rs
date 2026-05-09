//! Daemon's implementation of the transport `InboundHandler` trait.
//!
//! Thin forwarder: every inbound event is forwarded to the `SubscriptionManager`
//! for three-tier processing (state effects → Blake3 dedup → emit). RPC calls
//! are dispatched to community_rpc and governance_rpc handlers directly.
//!
//! `SubscriptionManager` owns all event emission via `SubscriptionEvent`.
//! Events flow through the IPC bus as `BusPayload::Event(SubscriptionEvent)`.


use std::sync::Arc;

use parking_lot::RwLock;
use tracing::{debug, info, warn};

use rekindle_transport::{
    InboundHandler, TransportEvent, VerifiedSender, SubscriptionManager,
    payload::dm::DmPayload,
    payload::gossip::{GossipPayload, SignedGossipEnvelope},
    payload::rpc::{CallResponse, InboundCall},
    payload::voice::VoicePayload,
    Session,
};

/// The daemon's inbound handler — thin forwarder to SubscriptionManager.
pub struct DaemonHandler {
    /// Subscription manager — all inbound events route through here.
    /// RwLock<Option<>> because it's None before resume, populated during unlock.
    pub(crate) subscriptions: Arc<RwLock<Option<SubscriptionManager>>>,
    /// Session state (shared with DaemonContext).
    pub(crate) session: Arc<RwLock<Option<Session>>>,
    /// Session file path for friend request persistence.
    pub(crate) session_path: std::path::PathBuf,
    /// MEK cache (shared with DaemonContext, passed to RPC handlers).
    pub(crate) mek_cache: Arc<RwLock<rekindle_transport::crypto::mek::MekCache>>,
    /// Signing key (shared with DaemonContext, passed to RPC handlers).
    pub(crate) signing_key: Arc<RwLock<Option<crate::state::keystore::SigningKeyHandle>>>,
    /// Transport node (shared with DaemonContext, passed to RPC handlers).
    pub(crate) transport: Arc<RwLock<Option<Arc<rekindle_transport::TransportNode>>>>,
    /// Friend inbox scan coordinator — non-blocking trigger for coalesced scans.
    pub(crate) inbox_scan: Arc<RwLock<Option<crate::daemon::friend_inbox::InboxScanCoordinator>>>,
    /// Pending community join completions (shared with DaemonContext).
    /// On JoinAccepted gossip, the handler completes the oneshot so the
    /// join handler unblocks immediately.
    pub(crate) pending_joins: Arc<parking_lot::Mutex<std::collections::HashMap<String, (tokio::sync::oneshot::Sender<u32>, std::time::Instant)>>>,
}

impl DaemonHandler {
    pub fn new(
        subscriptions: Arc<RwLock<Option<SubscriptionManager>>>,
        session: Arc<RwLock<Option<Session>>>,
        session_path: std::path::PathBuf,
        mek_cache: Arc<RwLock<rekindle_transport::crypto::mek::MekCache>>,
        signing_key: Arc<RwLock<Option<crate::state::keystore::SigningKeyHandle>>>,
        transport: Arc<RwLock<Option<Arc<rekindle_transport::TransportNode>>>>,
        inbox_scan: Arc<RwLock<Option<crate::daemon::friend_inbox::InboxScanCoordinator>>>,
        pending_joins: Arc<parking_lot::Mutex<std::collections::HashMap<String, (tokio::sync::oneshot::Sender<u32>, std::time::Instant)>>>,
    ) -> Self {
        Self { subscriptions, session, session_path, mek_cache, signing_key, transport, inbox_scan, pending_joins }
    }

    // persist_friend_request removed — friend requests are discovered
    // exclusively via DHT inbox scan (friend_inbox.rs), not app_message.
}

#[allow(clippy::manual_async_fn)]
impl InboundHandler for DaemonHandler {
    fn on_dm(
        &self, sender: &VerifiedSender, payload: DmPayload,
    ) -> impl std::future::Future<Output = ()> + Send {
        debug!(sender = &sender.public_key[..12.min(sender.public_key.len())], "handler: on_dm");

        // FriendRequestAck = "check your inbox" tier 2 notification.
        // Triggers an immediate inbox scan to discover the new request
        // from the SSOT (DHT inbox entry with signature + DM log key).
        let should_scan_inbox = matches!(payload, DmPayload::FriendRequestAck);

        // Forward to SubscriptionManager: into_event → state_effects → dedup → emit
        if let Some(ref sub_mgr) = *self.subscriptions.read() {
            sub_mgr.on_dm(&sender.public_key, payload);
        }

        let inbox_scan = Arc::clone(&self.inbox_scan);
        let session = Arc::clone(&self.session);

        async move {
            if !should_scan_inbox { return; }

            let inbox_key = {
                let guard = session.read();
                guard.as_ref().map(|s| s.identity.friend_inbox_key.clone()).unwrap_or_default()
            };
            if inbox_key.is_empty() { return; }

            debug!("FriendRequestAck received — triggering inbox scan");
            if let Some(ref coordinator) = *inbox_scan.read() {
                coordinator.trigger(&inbox_key);
            }
        }
    }

    fn on_gossip(
        &self, community_id: &str, sender_pseudonym: &str,
        payload: GossipPayload, lamport_ts: u64,
    ) -> impl std::future::Future<Output = ()> + Send {
        debug!(community = community_id, sender = &sender_pseudonym[..12.min(sender_pseudonym.len())], "handler: on_gossip");

        // Tier 2: If this is a JoinAccepted for a pending join, cache MEK + complete oneshot.
        // Check BEFORE forwarding to SubscriptionManager (which takes ownership).
        if let GossipPayload::Control(rekindle_transport::payload::gossip::ControlPayload::JoinAccepted {
            slot_index: Some(slot), ref mek_encrypted, mek_generation, ..
        }) = &payload {
            // Cache MEK from direct notification (bypasses DHT vault propagation)
            if !mek_encrypted.is_empty() && *mek_generation > 0 {
                if let Some(ref sk_handle) = *self.signing_key.read() {
                    let transfer = rekindle_transport::payload::rpc::MekTransferPayload {
                        channel_id: String::new(), // first channel — will be resolved by community governance
                        generation: *mek_generation,
                        rotator_pseudonym_hex: sender_pseudonym.to_string(),
                        wrapped_mek: mek_encrypted.clone(),
                    };
                    match rekindle_transport::operations::mek::receive_mek_transfer_payload(
                        &transfer, sk_handle.as_bytes(), community_id, &self.mek_cache,
                    ) {
                        Ok(_) => info!(community = community_id, generation = mek_generation, "MEK cached from JoinAccepted notification (tier 2)"),
                        Err(e) => debug!(community = community_id, error = %e, "MEK cache from notification failed — will read vault"),
                    }
                }
            }
            let mut pending = self.pending_joins.lock();
            if let Some((tx, _)) = pending.remove(community_id) {
                let _ = tx.send(*slot);
                info!(community = community_id, slot, "join approved via direct notification (tier 2)");
            }
        }

        if let Some(ref sub_mgr) = *self.subscriptions.read() {
            sub_mgr.on_gossip(community_id, sender_pseudonym, payload, lamport_ts);
        }
        async {}
    }

    fn on_gossip_forward(&self, _envelope: &SignedGossipEnvelope) -> impl std::future::Future<Output = ()> + Send {
        // Gossip forwarding to mesh peers — handled by broadcast manager
        async {}
    }

    fn on_voice(&self, _sender_key: &str, _packet: VoicePayload) -> impl std::future::Future<Output = ()> + Send {
        // Voice packet dispatch — handled by voice session manager
        async {}
    }

    fn on_call(
        &self, sender_pseudonym: Option<&str>, request: InboundCall,
    ) -> impl std::future::Future<Output = CallResponse> + Send {
        let mek_cache = Arc::clone(&self.mek_cache);
        let signing_key_arc = Arc::clone(&self.signing_key);
        let session_arc = Arc::clone(&self.session);
        let transport_arc = Arc::clone(&self.transport);
        let session_path = self.session_path.clone();
        let sender_ps = sender_pseudonym.map(String::from);

        async move {
            match request {
                InboundCall::CommunityLeave(notif) => {
                    super::community_rpc::handle_leave(
                        &notif, &session_arc, &signing_key_arc, &mek_cache, &transport_arc, &session_path,
                    ).await
                }
                InboundCall::CommunityGovOp(op) => {
                    super::governance_rpc::handle_op(
                        sender_ps.as_deref(), op, &session_arc, &signing_key_arc, &mek_cache, &transport_arc, &session_path,
                    ).await
                }
                InboundCall::Sync(_) | InboundCall::Dm(_) => CallResponse::Ack,
            }
        }
    }

    fn on_value_change(
        &self, record_key: &str, changed_subkeys: Vec<u32>, first_value: Option<Vec<u8>>,
    ) -> impl std::future::Future<Output = ()> + Send {
        debug!(record_key, subkeys = ?changed_subkeys, "handler: on_value_change");

        // Forward to SubscriptionManager for event emission
        if let Some(ref sub_mgr) = *self.subscriptions.read() {
            sub_mgr.on_value_change(record_key, changed_subkeys, first_value);
        }

        // Check if this is a join inbox for a community we operate.
        let governance_key = {
            let guard = self.session.read();
            guard.as_ref().and_then(|s| {
                s.communities.values()
                    .find(|m| m.is_operator && m.join_inbox_key == record_key)
                    .map(|m| m.governance_key.clone())
            })
        };

        // Check if this is our friend inbox.
        let is_friend_inbox = {
            let guard = self.session.read();
            guard.as_ref().is_some_and(|s| {
                !s.identity.friend_inbox_key.is_empty() && s.identity.friend_inbox_key == record_key
            })
        };

        let session = Arc::clone(&self.session);
        let signing_key = Arc::clone(&self.signing_key);
        let mek_cache = Arc::clone(&self.mek_cache);
        let transport = Arc::clone(&self.transport);
        let session_path = self.session_path.clone();
        let record_key_owned = record_key.to_string();
        let inbox_scan = Arc::clone(&self.inbox_scan);

        async move {
            // Process community join inbox
            if let Some(gov_key) = governance_key {
                info!(governance_key = %gov_key, "join inbox changed — processing");
                super::community_rpc::process_inbox(
                    &session, &signing_key, &mek_cache, &transport, &session_path, &gov_key,
                ).await;
            }

            // Trigger friend inbox scan via coordinator (non-blocking, coalesced)
            if is_friend_inbox {
                debug!("friend inbox changed — triggering scan via coordinator");
                if let Some(ref coordinator) = *inbox_scan.read() {
                    coordinator.trigger(&record_key_owned);
                }
            }
        }
    }

    fn on_event(&self, event: TransportEvent) -> impl std::future::Future<Output = ()> + Send {
        debug!(event = ?std::mem::discriminant(&event), "handler: on_event");

        // Extract what we need from behind the lock, then drop it before any await.
        // Synchronous events (route deaths) are handled inline.
        // Async events (attachment change, watch death) are handled after lock drop.
        let async_work = {
            let guard = self.subscriptions.read();
            if let Some(ref sub_mgr) = *guard {
                match &event {
                    TransportEvent::LocalRoutesDied { count } => {
                        sub_mgr.on_route_change(*count, vec![]);
                        None
                    }
                    TransportEvent::RemoteRoutesDied { peer_keys } => {
                        sub_mgr.on_route_change(0, peer_keys.clone());
                        None
                    }
                    TransportEvent::AttachmentChanged { .. }
                    | TransportEvent::WatchDied { .. } => {
                        // Need async work — extract Arcs for post-lock operations
                        Some((
                            Arc::clone(sub_mgr.node()),
                            Arc::clone(sub_mgr.watches()),
                            sub_mgr.event_sender().clone(),
                        ))
                    }
                }
            } else {
                None
            }
        };
        // Guard dropped — safe to await

        async move {
            let Some((node, watches, event_tx)) = async_work else { return; };

            match event {
                TransportEvent::AttachmentChanged { is_attached, public_internet_ready, .. } => {
                    // Emit attachment change event
                    let _ = event_tx.send(rekindle_types::subscription_events::SubscriptionEvent::Network(
                        rekindle_types::subscription_events::NetworkEvent::AttachmentChanged {
                            is_attached, public_internet_ready,
                        },
                    ));

                    // Re-establish ALL watches on network re-attach — not just stale ones.
                    // After a 30-second outage, no watches are past their 4-minute renewal
                    // interval, so needs_renewal() would return empty. Every watch must be
                    // renewed because the remote storage nodes may have expired them during
                    // the outage regardless of our local renewal timestamp.
                    if is_attached && public_internet_ready {
                        let all_watches: Vec<(String, _)> = watches.read().entries.iter()
                            .map(|(k, e)| (k.clone(), e.clone()))
                            .collect();
                        info!(count = all_watches.len(), "network re-attached — re-establishing all watches");
                        for (record_key, entry) in all_watches {
                            if rekindle_transport::subscriptions::watches::renew_watch(
                                &node, &record_key, &entry.subkeys,
                            ).await {
                                if let Some(e) = watches.write().entries.get_mut(&record_key) {
                                    e.established_at = std::time::Instant::now();
                                }
                            }
                        }
                    }
                }
                TransportEvent::WatchDied { record_key, .. } => {
                    // Immediate re-establishment — don't wait for the 60s renewal tick
                    let entry = watches.read().get(&record_key).cloned();
                    if let Some(entry) = entry {
                        info!(record_key = %record_key, "watch died — re-establishing immediately");
                        if rekindle_transport::subscriptions::watches::renew_watch(
                            &node, &record_key, &entry.subkeys,
                        ).await {
                            if let Some(e) = watches.write().entries.get_mut(&record_key) {
                                e.established_at = std::time::Instant::now();
                            }
                            let _ = event_tx.send(rekindle_types::subscription_events::SubscriptionEvent::Network(
                                rekindle_types::subscription_events::NetworkEvent::WatchReestablished {
                                    record_key,
                                },
                            ));
                        } else {
                            warn!(record_key = %record_key, "watch re-establishment failed after death");
                            let _ = event_tx.send(rekindle_types::subscription_events::SubscriptionEvent::Network(
                                rekindle_types::subscription_events::NetworkEvent::WatchFailed {
                                    record_key,
                                    error: "re-establishment failed after watch death".into(),
                                },
                            ));
                        }
                    }
                }
                _ => {} // LocalRoutesDied and RemoteRoutesDied already handled synchronously above
            }
        }
    }
}
