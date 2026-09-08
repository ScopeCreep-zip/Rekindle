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
    /// `WatchRelay` is handled here because it has no `GossipPayload`
    /// equivalent — the daemon's three-variant type cannot express it.
    /// Everything that *does* have one is projected and handed to the
    /// existing `on_gossip` pipeline rather than reimplemented, so
    /// unread counts, typing state and the presence overlay behave
    /// identically whichever format the message arrived in.
    async fn on_community_gossip(
        &self,
        community_id: &str,
        sender_pseudonym: &str,
        envelope: rekindle_protocol::dht::community::envelope::CommunityEnvelope,
        lamport_ts: u64,
    ) {
        use rekindle_protocol::dht::community::envelope::CommunityEnvelope as Env;

        debug!(
            community = community_id,
            sender = &sender_pseudonym[..12.min(sender_pseudonym.len())],
            "handler: on_community_gossip"
        );

        let projected = match envelope {
            Env::WatchRelay {
                record_key,
                subkey,
                content_hash,
                observer_pseudonym,
            } => {
                self.on_watch_relay(&record_key, subkey, &content_hash, &observer_pseudonym)
                    .await;
                return;
            }
            Env::MessageNotification {
                channel_id,
                message_id,
                author_pseudonym,
                subkey_index,
                lamport_ts: inner_lamport,
                sequence,
                content_hash,
                timestamp,
            } => GossipPayload::MessageNotification {
                channel_id,
                message_id,
                author_pseudonym,
                subkey_index,
                lamport_ts: inner_lamport,
                sequence,
                content_hash,
                timestamp,
            },
            Env::TypingIndicator {
                channel_id,
                pseudonym_key,
            } => GossipPayload::TypingIndicator {
                channel_id,
                pseudonym_key,
            },
            Env::PresenceUpdate {
                pseudonym_key,
                status,
                game_info,
                route_blob,
            } => GossipPayload::PresenceUpdate {
                pseudonym_key,
                status,
                // `PresenceGameInfo` is one struct on the protocol side
                // and three flat fields on the transport side. Unpacked
                // rather than dropped: rich presence is the whole point
                // of the field, and losing it silently would make a
                // desktop peer's game status vanish on the daemon.
                game_name: game_info.as_ref().map(|g| g.game_name.clone()),
                game_id: game_info.as_ref().and_then(|g| g.game_id),
                elapsed_seconds: game_info.as_ref().and_then(|g| g.elapsed_seconds),
                server_address: game_info.as_ref().and_then(|g| g.server_address.clone()),
                route_blob,
            },
            // The `Control(..)` family. Both tracks have one, but they
            // are separate enums with different variant sets, so a
            // faithful projection is a per-variant mapping rather than
            // a cast — and writing a lossy one here would silently drop
            // whichever control messages did not line up.
            //
            // Logged rather than dropped quietly: this is a known gap
            // with a name, and the traffic that reaches it is visible
            // under `RUST_LOG=rekindle_node=debug`.
            Env::Control(control) => {
                debug!(
                    community = community_id,
                    variant = ?std::mem::discriminant(&control),
                    "on_community_gossip: control variant not yet projected onto the daemon's \
                     ControlPayload"
                );
                return;
            }
        };

        self.on_gossip(community_id, sender_pseudonym, projected, lamport_ts)
            .await;
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
