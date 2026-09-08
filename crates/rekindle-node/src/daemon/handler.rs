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
    payload::dm::DmPayload,
    payload::gossip::{GossipPayload, SignedGossipEnvelope},
    payload::rpc::{CallResponse, InboundCall},
    payload::voice::VoicePayload,
    InboundHandler, PendingFriendRequest, Session, SubscriptionManager, TransportEvent,
    VerifiedSender,
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
    /// Pending community join completions (shared with DaemonContext).
    /// On JoinAccepted gossip, the handler completes the oneshot so the
    /// join handler unblocks immediately.
    pub(crate) pending_joins: Arc<
        parking_lot::Mutex<
            std::collections::HashMap<
                String,
                (tokio::sync::oneshot::Sender<u32>, std::time::Instant),
            >,
        >,
    >,
    /// Queues departure-triggered MEK rotations. The leave
    /// notification arrives here, and under v2.0 the only correct
    /// response is to start the deterministic rotation.
    pub(crate) mek_rotation_tx: crate::daemon::mek_rotation::MekRotationSender,
    /// Outbound gossip, for the Mutual Aid watch relay (§14.3).
    pub(crate) gossip_tx: crate::daemon::gossip::GossipSender,
}

impl DaemonHandler {
    pub fn new(
        subscriptions: Arc<RwLock<Option<SubscriptionManager>>>,
        session: Arc<RwLock<Option<Session>>>,
        session_path: std::path::PathBuf,
        mek_cache: Arc<RwLock<rekindle_transport::crypto::mek::MekCache>>,
        signing_key: Arc<RwLock<Option<crate::state::keystore::SigningKeyHandle>>>,
        transport: Arc<RwLock<Option<Arc<rekindle_transport::TransportNode>>>>,
        pending_joins: Arc<
            parking_lot::Mutex<
                std::collections::HashMap<
                    String,
                    (tokio::sync::oneshot::Sender<u32>, std::time::Instant),
                >,
            >,
        >,
        mek_rotation_tx: crate::daemon::mek_rotation::MekRotationSender,
        gossip_tx: crate::daemon::gossip::GossipSender,
    ) -> Self {
        Self {
            subscriptions,
            session,
            session_path,
            mek_cache,
            signing_key,
            transport,
            pending_joins,
            mek_rotation_tx,
            gossip_tx,
        }
    }

    /// Persist a friend request to session before forwarding to SubscriptionManager.
    /// This must happen synchronously before the event pipeline because the session
    /// state is read by the subscription manager's state_effects.
    fn persist_friend_request(
        &self,
        sender_key: &str,
        display_name: &str,
        message: &str,
        profile_dht_key: &str,
        route_blob: &[u8],
        mailbox_dht_key: &str,
        prekey_bundle: &[u8],
        invite_id: Option<&String>,
        timestamp: u64,
    ) {
        let pending = PendingFriendRequest {
            public_key: sender_key.to_string(),
            display_name: display_name.to_string(),
            message: message.to_string(),
            profile_dht_key: profile_dht_key.to_string(),
            route_blob: route_blob.to_vec(),
            mailbox_dht_key: mailbox_dht_key.to_string(),
            prekey_bundle: prekey_bundle.to_vec(),
            invite_id: invite_id.cloned(),
            received_at: timestamp,
        };
        let mut guard = self.session.write();
        if let Some(ref mut session) = *guard {
            session.add_pending_friend_request(pending);
            if let Err(e) = session.save(&self.session_path) {
                warn!(error = %e, "failed to persist pending friend request");
            } else {
                info!(
                    from = sender_key,
                    name = display_name,
                    "friend request persisted"
                );
            }
        }
    }
}

impl InboundHandler for DaemonHandler {
    async fn on_dm(
        &self,
        sender: &VerifiedSender,
        payload: DmPayload,
        timestamp: u64,
        _seq: u64,
        _correlation_id: Option<&str>,
    ) {
        debug!(
            sender = &sender.public_key[..12.min(sender.public_key.len())],
            "handler: on_dm"
        );

        // Persist friend requests to session before event pipeline
        if let DmPayload::FriendRequest {
            ref display_name,
            ref message,
            ref prekey_bundle,
            ref profile_dht_key,
            ref route_blob,
            ref mailbox_dht_key,
            ref invite_id,
        } = payload
        {
            self.persist_friend_request(
                &sender.public_key,
                display_name,
                message,
                profile_dht_key,
                route_blob,
                mailbox_dht_key,
                prekey_bundle,
                invite_id.as_ref(),
                timestamp,
            );
        }

        // If we receive a FriendRequestAck, it means someone wrote to our friend
        // inbox. Trigger an immediate inbox scan to discover the new request.
        let should_scan_inbox = matches!(payload, DmPayload::FriendRequestAck);

        // Forward to SubscriptionManager: into_event → state_effects → dedup → emit
        if let Some(ref sub_mgr) = *self.subscriptions.read() {
            sub_mgr.on_dm(&sender.public_key, payload, timestamp);
        }

        let session = Arc::clone(&self.session);
        let transport = Arc::clone(&self.transport);
        let session_path = self.session_path.clone();

        if !should_scan_inbox {
            return;
        }

        let inbox_key = {
            let guard = session.read();
            guard
                .as_ref()
                .map(|s| s.identity.friend_inbox_key.clone())
                .unwrap_or_default()
        };
        if inbox_key.is_empty() {
            return;
        }

        debug!("FriendRequestAck received — scanning friend inbox");
        super::friend_inbox::scan_friend_inbox(&session, &transport, &session_path, &inbox_key)
            .await;
    }

    /// A desktop-format community gossip envelope arrived, verified.
    ///
    /// Routed by variant here rather than projected onto
    /// `GossipPayload`: `CommunityEnvelope` carries the whole
    /// `Control(..)` family and `WatchRelay`, none of which the
    /// three-variant `GossipPayload` can express.
    async fn on_community_gossip(
        &self,
        community_id: &str,
        sender_pseudonym: &str,
        envelope: rekindle_protocol::dht::community::envelope::CommunityEnvelope,
        _lamport_ts: u64,
    ) {
        use rekindle_protocol::dht::community::envelope::CommunityEnvelope as Env;

        debug!(
            community = community_id,
            sender = &sender_pseudonym[..12.min(sender_pseudonym.len())],
            "handler: on_community_gossip"
        );

        match envelope {
            Env::WatchRelay {
                record_key,
                subkey,
                content_hash,
                observer_pseudonym,
            } => {
                self.on_watch_relay(&record_key, subkey, &content_hash, &observer_pseudonym)
                    .await;
            }
            // Everything else is delivered as a value-change style
            // signal so the existing subscription pipeline picks it up.
            // Deliberately not silent: an unrouted variant is a gap to
            // close, not traffic to ignore.
            other => {
                debug!(
                    community = community_id,
                    variant = ?std::mem::discriminant(&other),
                    "on_community_gossip: variant not yet routed on this track"
                );
            }
        }
    }

    async fn on_gossip(
        &self,
        community_id: &str,
        sender_pseudonym: &str,
        payload: GossipPayload,
        lamport_ts: u64,
    ) {
        debug!(
            community = community_id,
            sender = &sender_pseudonym[..12.min(sender_pseudonym.len())],
            "handler: on_gossip"
        );

        // Tier 2: If this is a JoinAccepted for a pending join, cache MEK + complete oneshot.
        // Check BEFORE forwarding to SubscriptionManager (which takes ownership).
        if let GossipPayload::Control(
            rekindle_transport::payload::gossip::ControlPayload::JoinAccepted {
                slot_index: Some(slot),
                ref mek_encrypted,
                mek_generation,
                ..
            },
        ) = &payload
        {
            // Cache MEK from direct notification (bypasses DHT vault propagation)
            if !mek_encrypted.is_empty() && *mek_generation > 0 {
                if let Some(ref sk_handle) = *self.signing_key.read() {
                    let transfer = rekindle_transport::payload::rpc::MekTransferPayload {
                        channel_id: String::new(), // first channel — will be resolved by community governance
                        generation: *mek_generation,
                        rotator_pseudonym_hex: String::new(),
                        wrapped_mek: mek_encrypted.clone(),
                    };
                    match rekindle_transport::operations::mek::receive_mek_transfer_payload(
                        &transfer,
                        sk_handle.as_bytes(),
                        community_id,
                        &self.mek_cache,
                    ) {
                        Ok(_) => info!(
                            community = community_id,
                            generation = mek_generation,
                            "MEK cached from JoinAccepted notification (tier 2)"
                        ),
                        Err(e) => {
                            debug!(community = community_id, error = %e, "MEK cache from notification failed — will read vault");
                        }
                    }
                }
            }
            let mut pending = self.pending_joins.lock();
            if let Some((tx, _)) = pending.remove(community_id) {
                let _ = tx.send(*slot);
                info!(
                    community = community_id,
                    slot, "join approved via direct notification (tier 2)"
                );
            }
        }

        if let Some(ref sub_mgr) = *self.subscriptions.read() {
            sub_mgr.on_gossip(community_id, sender_pseudonym, payload, lamport_ts);
        }
    }

    async fn on_gossip_forward(&self, _envelope: &SignedGossipEnvelope) {
        // Gossip forwarding to mesh peers — handled by broadcast manager
    }

    async fn on_voice(&self, _sender_key: &str, _packet: VoicePayload) {
        // Voice packet dispatch — handled by voice session manager
    }

    async fn on_call(&self, sender_pseudonym: Option<&str>, request: InboundCall) -> CallResponse {
        let mek_cache = Arc::clone(&self.mek_cache);
        let signing_key_arc = Arc::clone(&self.signing_key);
        let session_arc = Arc::clone(&self.session);
        let transport_arc = Arc::clone(&self.transport);
        let session_path = self.session_path.clone();
        let sender_ps = sender_pseudonym.map(String::from);

        match request {
            InboundCall::CommunityLeave(notif) => {
                super::community_rpc::handle_leave(&notif, &session_arc, &self.mek_rotation_tx)
            }
            InboundCall::CommunityGovOp(op) => {
                super::governance_rpc::handle_op(
                    sender_ps.as_deref(),
                    op,
                    &session_arc,
                    &transport_arc,
                    &session_path,
                )
                .await
            }
            InboundCall::CommunityMekTransfer(transfer) => {
                super::community_rpc::handle_mek_transfer(
                    &transfer,
                    &session_arc,
                    &signing_key_arc,
                    &mek_cache,
                )
            }
            InboundCall::Sync(_) | InboundCall::Dm(_) => CallResponse::Ack,
            InboundCall::CallInvite(invite) => {
                // W16.5b — the daemon shell doesn't yet host a
                // `CallRuntime` (calls are GUI features wired via
                // the Tauri shell). Reply with a typed Rejected so
                // the caller's UI surfaces "Couldn't reach {peer}"
                // via `CallUnreachable { reason: "send_failed" }`
                // — equivalent to the receiver's call capability
                // being absent. W16.18 wires the CallRuntime here
                // for cross-shell parity.
                debug!(
                    call_id = %invite.call_id,
                    sender = ?sender_ps,
                    "InboundCall::CallInvite at daemon — no CallRuntime; rejecting"
                );
                CallResponse::Rejected {
                    reason: "daemon has no call runtime".into(),
                }
            }
        }
    }

    async fn on_value_change(
        &self,
        record_key: &str,
        changed_subkeys: Vec<u32>,
        first_value: Option<Vec<u8>>,
    ) {
        debug!(record_key, subkeys = ?changed_subkeys, "handler: on_value_change");

        // Mutual Aid §14.3 — pay the watch slot forward.
        //
        // Veilid reserves 8 signed + 32 anonymous watch slots per
        // record, so in a community of any size most members hold none.
        // Holding one makes us an eager-push node in Plumtree terms, and
        // the obligation that comes with it is to lazy-push what we saw
        // to everyone who could not get a slot. Principle 12: "a member
        // who relays for peers gets relayed for in return."
        //
        // Only for a change we observed through our own watch — a relay
        // we ourselves fetched from a peer's relay must not be
        // re-relayed, or one change echoes around the mesh.
        self.relay_watch_change(record_key, &changed_subkeys, first_value.as_deref());

        // Forward to SubscriptionManager for event emission
        if let Some(ref sub_mgr) = *self.subscriptions.read() {
            sub_mgr.on_value_change(record_key, changed_subkeys, first_value);
        }

        // Check if this is our friend inbox.
        let is_friend_inbox = {
            let guard = self.session.read();
            guard.as_ref().is_some_and(|s| {
                !s.identity.friend_inbox_key.is_empty() && s.identity.friend_inbox_key == record_key
            })
        };

        let session = Arc::clone(&self.session);
        let transport = Arc::clone(&self.transport);
        let session_path = self.session_path.clone();
        let record_key_owned = record_key.to_string();

        // Process friend inbox — scan for new requests and persist to session
        if is_friend_inbox {
            debug!("friend inbox changed — scanning for new requests");
            super::friend_inbox::scan_friend_inbox(
                &session,
                &transport,
                &session_path,
                &record_key_owned,
            )
            .await;
        }
    }

    async fn on_event(&self, event: TransportEvent) {
        debug!(event = ?std::mem::discriminant(&event), "handler: on_event");
        if let Some(ref sub_mgr) = *self.subscriptions.read() {
            match event {
                TransportEvent::AttachmentChanged {
                    is_attached,
                    public_internet_ready,
                    ..
                } => {
                    sub_mgr.on_route_change(0, vec![]); // triggers NetworkStateChanged render
                    let _ = (is_attached, public_internet_ready); // used by attachment handler
                }
                TransportEvent::LocalRoutesDied { count } => {
                    sub_mgr.on_route_change(count, vec![]);
                }
                TransportEvent::RemoteRoutesDied { peer_keys } => {
                    sub_mgr.on_route_change(0, peer_keys);
                }
                TransportEvent::WatchDied { .. } => {
                    // Watch re-establishment handled by the renewal loop in SubscriptionManager
                }
            }
        }
    }
}

impl DaemonHandler {
    /// Mutual Aid §14.3 — a peer holding a watch slot is telling us a
    /// record's subkey changed.
    ///
    /// This is Plumtree's lazy push. Veilid reserves only
    /// `member_watch_limit` (8) signed watch slots plus
    /// `public_watch_limit` (32) anonymous ones **per record**, so in a
    /// community of any size most members hold no watch on any given
    /// record. Peers that do hold one relay the notification — the
    /// `IHAVE` — and we pull the value ourselves.
    ///
    /// The relay carries a `content_hash` and no ciphertext, on purpose:
    /// gossip is unencrypted at the envelope layer, so shipping the
    /// value would leak it to every hop. The hash lets us verify that
    /// what we fetched is what the observer saw.
    async fn on_watch_relay(
        &self,
        record_key: &str,
        subkey: u32,
        content_hash: &str,
        observer_pseudonym: &str,
    ) {
        let Some(transport) = self.transport.read().clone() else {
            return;
        };

        // If we hold our own watch on this record, our value-change
        // callback covers the same change — skip the redundant fetch.
        // Members without a slot fall through, which is the whole point
        // of the relay.
        let we_watch = self
            .subscriptions
            .read()
            .as_ref()
            .is_some_and(|manager| manager.has_watch(record_key));
        if we_watch {
            tracing::trace!(
                record_key,
                subkey,
                "watch relay: own watch covers this record"
            );
            return;
        }

        let Ok(Some(value)) = rekindle_transport::broadcast::dht_writes::get(
            transport.as_ref(),
            record_key,
            subkey,
            true,
        )
        .await
        else {
            return;
        };

        let actual = blake3::hash(&value).to_hex().to_string();
        if actual != content_hash {
            debug!(
                record_key,
                subkey,
                observer = &observer_pseudonym[..12.min(observer_pseudonym.len())],
                "watch relay: content hash mismatch, dropping"
            );
            return;
        }

        self.on_value_change(record_key, vec![subkey], Some(value))
            .await;
    }
}

impl DaemonHandler {
    /// Broadcast a `WatchRelay` for a change our own watch reported.
    ///
    /// The relay names the record, the subkey and a BLAKE3 hash of the
    /// new value — never the value. Gossip is unencrypted at the
    /// envelope layer, so shipping the bytes would hand a channel
    /// message's ciphertext to every forwarding hop; the hash is what a
    /// watchless peer verifies its own fetch against.
    ///
    /// Silent when we hold no watch on the record: `on_value_change`
    /// also fires for values we pulled after someone else's relay, and
    /// re-relaying those would put one change into an endless loop
    /// around the mesh (the dedup cache would break the loop, but only
    /// after the traffic had gone out).
    fn relay_watch_change(&self, record_key: &str, changed_subkeys: &[u32], value: Option<&[u8]>) {
        let Some(value) = value else {
            // No first value means either a multi-subkey change (the
            // caller must fetch each one anyway) or a dead watch. In
            // both cases we have no hash to publish.
            return;
        };
        let Some(subkey) = changed_subkeys.first().copied() else {
            return;
        };
        let we_watch = self
            .subscriptions
            .read()
            .as_ref()
            .is_some_and(|manager| manager.has_watch(record_key));
        if !we_watch {
            return;
        }

        // Which community does this record belong to, and who are we in
        // it? A relay has to be attributable — `observer_pseudonym` is
        // what lets a receiver weigh it.
        let Some((community_id, observer)) = self.community_for_record(record_key) else {
            return;
        };

        let envelope = rekindle_protocol::dht::community::envelope::CommunityEnvelope::WatchRelay {
            record_key: record_key.to_string(),
            subkey,
            content_hash: blake3::hash(value).to_hex().to_string(),
            observer_pseudonym: observer,
        };
        crate::daemon::gossip::send(&self.gossip_tx, &community_id, &envelope);
    }

    /// `(community_id, my_pseudonym)` for whichever community owns this
    /// record — governance, registry, or one of its channel records.
    fn community_for_record(&self, record_key: &str) -> Option<(String, String)> {
        let guard = self.session.read();
        let session = guard.as_ref()?;
        session.communities.values().find_map(|m| {
            let ours = m.governance_key == record_key
                || m.registry_key == record_key
                || m.channel_record_keys.values().any(|k| k == record_key);
            (ours && !m.pseudonym_key.is_empty())
                .then(|| (m.governance_key.clone(), m.pseudonym_key.clone()))
        })
    }
}
