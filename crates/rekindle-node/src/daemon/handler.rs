//! Daemon's implementation of the transport `InboundHandler` trait.
//!
//! Thin forwarder: every inbound event is forwarded to the `SubscriptionManager`
//! for three-tier processing (state effects → Blake3 dedup → emit). RPC calls
//! are dispatched to community_rpc and governance_rpc handlers directly; 1:1
//! messages and friend presence run through the Host's friend operations
//! (`operations::direct`, plan E2.1), which publish to the journal.
//!
//! `SubscriptionManager` owns all event emission via `SubscriptionEvent`.
//! Events flow through the IPC bus as `BusPayload::Event(SubscriptionEvent)`.

use std::sync::Arc;

mod gossip_ingest;

use parking_lot::RwLock;
use rekindle_codec::community::envelope::{CommunityEnvelope, SignedEnvelope};
use tracing::debug;

use rekindle_transport::operations::direct::{self, Routed};
use rekindle_transport::{
    payload::rpc::{CallResponse, InboundCall},
    InboundHandler, Session, SubscriptionManager, TransportEvent,
};

/// The daemon's inbound handler — thin forwarder to SubscriptionManager.
pub struct DaemonHandler {
    /// Subscription manager — all inbound events route through here.
    /// RwLock<Option<>> because it's None before resume, populated during unlock.
    pub(crate) subscriptions: Arc<RwLock<Option<SubscriptionManager>>>,
    /// Session state (shared with DaemonContext).
    pub(crate) session: Arc<RwLock<Option<Session>>>,
    /// Signing key (shared with DaemonContext, passed to RPC handlers).
    pub(crate) signing_key: Arc<RwLock<Option<crate::state::signing_key::SigningKeyHandle>>>,
    /// Transport node, weak: the node's dispatch loop owns this handler, so
    /// a strong reference here is a cycle that kept the node from being
    /// shut down at exit (A/B R1-16 "outstanding references").
    pub(crate) transport: Arc<RwLock<Option<std::sync::Weak<rekindle_transport::TransportNode>>>>,
    /// Queues departure-triggered MEK rotations. The leave
    /// notification arrives here, and under v2.0 the only correct
    /// response is to start the deterministic rotation.
    pub(crate) mek_rotation_tx: crate::daemon::mek_rotation::MekRotationSender,
    /// Outbound gossip, for the Mutual Aid watch relay (§14.3).
    pub(crate) gossip_tx: crate::daemon::gossip::GossipSender,
    /// The host's wall clock (plan C9.2).
    pub(crate) clock: Arc<dyn rekindle_utils::Clock>,
    /// The host this handler was built before, set once it exists: 1:1
    /// messages and friend presence run through its friend operations
    /// (plan E2.1).
    pub(crate) host:
        Arc<std::sync::OnceLock<std::sync::Weak<crate::daemon::dispatch::DaemonContext>>>,
    /// The network indicator's smoothed bars (plan E7.2, live finding L8).
    strength: Arc<rekindle_events::strength::Strength>,
}

impl DaemonHandler {
    pub fn new(
        subscriptions: Arc<RwLock<Option<SubscriptionManager>>>,
        session: Arc<RwLock<Option<Session>>>,
        signing_key: Arc<RwLock<Option<crate::state::signing_key::SigningKeyHandle>>>,
        transport: Arc<RwLock<Option<std::sync::Weak<rekindle_transport::TransportNode>>>>,
        mek_rotation_tx: crate::daemon::mek_rotation::MekRotationSender,
        gossip_tx: crate::daemon::gossip::GossipSender,
        clock: Arc<dyn rekindle_utils::Clock>,
        host: Arc<std::sync::OnceLock<std::sync::Weak<crate::daemon::dispatch::DaemonContext>>>,
        strength: Arc<rekindle_events::strength::Strength>,
    ) -> Self {
        Self {
            subscriptions,
            session,
            signing_key,
            transport,
            mek_rotation_tx,
            gossip_tx,
            clock,
            host,
            strength,
        }
    }

    /// Veilid's attachment changed to `state`: report it to the indicator
    /// and publish `NetworkEvent::Strength` when the shown bars change; a
    /// held fall is re-read once its hold ends (plan E7.2, L8).
    fn sample_strength(&self, state: &str) {
        let report = self.strength.report(state);
        if let Some(bars) = report.publish {
            publish_strength(&self.subscriptions, bars);
        }
        // Before unlock there is no Host scope and no subscriber; the next
        // report re-reads the window.
        let scope = self
            .host
            .get()
            .and_then(std::sync::Weak::upgrade)
            .map(|ctx| ctx.unlock_scope_or_closed());
        if let (true, Some(scope)) = (report.hold, scope) {
            let strength = Arc::clone(&self.strength);
            let subscriptions = Arc::clone(&self.subscriptions);
            scope.spawn_or_drop("network strength hold", async move {
                if let Some(bars) = strength.settle_held().await {
                    publish_strength(&subscriptions, bars);
                }
            });
        }
    }

    /// The friend operations' Host, once the host exists and while it lives.
    fn friends(&self) -> Option<crate::host::adapters::friends::HostFriends> {
        let ctx = self.host.get()?.upgrade()?;
        Some(crate::host::adapters::friends::HostFriends::of(&ctx))
    }

    /// Whether the Host accepts writes now (its lifecycle's `can_write`),
    /// for the network status a window gates writes on (plan E7.13).
    fn can_write(&self) -> bool {
        self.host
            .get()
            .and_then(std::sync::Weak::upgrade)
            .is_some_and(|ctx| ctx.lifecycle.state().can_write())
    }

    /// Whether we hold a personal route, and what the media route is doing
    /// (plan C7.9c): the route facts the network status carries. Route
    /// state lives on the transport's route owner, not the manager.
    fn route_status(&self) -> (bool, rekindle_types::subscription_events::RouteAvailability) {
        use rekindle_records::port::RouteClass;
        let routes = self
            .transport
            .read()
            .as_ref()
            .and_then(std::sync::Weak::upgrade)
            .map(|t| t.routes());
        let Some(routes) = routes else {
            return (
                false,
                rekindle_types::subscription_events::RouteAvailability::Idle,
            );
        };
        let media = routes.own_state(RouteClass::Media).borrow().availability();
        (routes.own_blob(RouteClass::General).is_some(), media)
    }
}

impl InboundHandler for DaemonHandler {
    fn local_identity(&self) -> Option<[u8; 32]> {
        let guard = self.session.read();
        let hex = &guard.as_ref()?.identity.public_key_hex;
        rekindle_transport::recipient_bytes(hex).ok()
    }

    fn clock(&self) -> &dyn rekindle_utils::Clock {
        self.clock.as_ref()
    }

    async fn on_direct(&self, datagram: &[u8]) {
        let Some(host) = self.friends() else {
            debug!("1:1 message before the host is up — dropped");
            return;
        };
        if let Routed::Unported(opened) = direct::receive(&host, datagram).await {
            // The daemon has none of the unported features (calls, relays)
            // yet; their owner steps bring them.
            debug!(from = %opened.sender_hex, kind = ?opened.type_id, "unported 1:1 kind dropped at the daemon");
        }
    }

    async fn on_direct_call(&self, datagram: &[u8]) -> Option<Vec<u8>> {
        let host = self.friends()?;
        match direct::answer_call(&host, datagram).await {
            direct::Called::Answered(reply) => Some(reply),
            direct::Called::Unported(opened) => {
                // The daemon relays for no one yet (E5.2): refused.
                debug!(from = %opened.sender_hex, "unported 1:1 call refused at the daemon");
                Some(direct::NAK.to_vec())
            }
            direct::Called::RetryLater => Some(direct::RETRY_LATER.to_vec()),
        }
    }

    async fn on_gossip(
        &self,
        community_id: &str,
        sender_pseudonym: &str,
        envelope: CommunityEnvelope,
        lamport_ts: u64,
    ) {
        self.ingest_gossip(community_id, sender_pseudonym, envelope, lamport_ts)
            .await;
    }

    /// Re-broadcast a verified envelope one hop further.
    ///
    /// Queued for the gossip worker, which re-signs nothing — the
    /// envelope keeps its original author's signature, and only the TTL
    /// the transport already decremented has changed. That is what makes
    /// a relayed message still attributable to whoever wrote it.
    async fn on_gossip_forward(&self, envelope: &SignedEnvelope) {
        if !self.relays(&envelope.community_id, &envelope.sender_pseudonym) {
            return;
        }
        crate::daemon::gossip::forward(&self.gossip_tx, envelope.clone());
    }

    /// A ring watcher's hint (plan E3.2 S7, `headAdvanced @76`): the
    /// member gate, then the sender's rate limit, then the hint's own
    /// signature; the verified read runs after, off the relay.
    async fn on_head_advanced(
        &self,
        community_id: &str,
        sender_pseudonym: &str,
        hint: &[u8],
    ) -> bool {
        let Some(ctx) = self.host.get().and_then(std::sync::Weak::upgrade) else {
            return false;
        };
        let governance = crate::host::adapters::governance::HostGovernance::new(ctx);
        let admit = |sender: &str| match *self.subscriptions.read() {
            Some(ref sub_mgr) => sub_mgr.admit(community_id, sender, 0),
            None => false,
        };
        governance.on_head_advanced(community_id, sender_pseudonym, hint, admit)
    }

    async fn on_call(&self, _sender_pseudonym: Option<&str>, request: InboundCall) -> CallResponse {
        match request {
            InboundCall::CommunityLeave(notif) => {
                super::community_rpc::handle_leave(&notif, &self.session, &self.mek_rotation_tx)
            }
            InboundCall::CommunityMekTransfer(transfer) => {
                let governance = self
                    .host
                    .get()
                    .and_then(std::sync::Weak::upgrade)
                    .map(crate::host::adapters::governance::HostGovernance::new);
                super::community_rpc::handle_mek_transfer(
                    &transfer,
                    &self.session,
                    &self.signing_key,
                    governance.as_ref(),
                )
            }
            InboundCall::BootstrapRequest {
                joiner_pseudonym,
                anchor,
                member_joined,
                member_joined_op,
                joiner_route,
            } => {
                let Some(ctx) = self.host.get().and_then(std::sync::Weak::upgrade) else {
                    return CallResponse::Rejected {
                        reason: "no host".into(),
                    };
                };
                let joiner = rekindle_governance_runtime::BootstrapJoiner {
                    pseudonym: joiner_pseudonym,
                    member_joined,
                    member_joined_op,
                    route: joiner_route,
                };
                let deps = Arc::new(super::mek_rotation::DaemonMekAdapter::new(Arc::clone(&ctx)));
                let host = crate::host::adapters::governance::HostGovernance::new(ctx);
                match rekindle_governance_runtime::answer_bootstrap_request(&host, &anchor, &joiner)
                    .await
                {
                    Ok(rekindle_governance_runtime::BootstrapAnswer::Frontier(bytes)) => {
                        host.deliver_key_to_joiner(deps.as_ref(), &anchor, &joiner)
                            .await;
                        CallResponse::Ok(bytes)
                    }
                    // Partial (S7c.8 (i)): the typed answer, never a
                    // refusal; the joiner asks another member.
                    Ok(rekindle_governance_runtime::BootstrapAnswer::Partial(bytes)) => {
                        CallResponse::Ok(bytes)
                    }
                    // The merge does not admit the join: the typed
                    // refusal, so the joiner can say why (plan E3.2).
                    Err(rekindle_governance_runtime::GovernanceRuntimeError::JoinNotAdmitted(
                        reason,
                    )) => match rekindle_governance_runtime::encode_refusal(&anchor, reason) {
                        Ok(bytes) => CallResponse::Ok(bytes),
                        Err(e) => CallResponse::Rejected {
                            reason: e.to_string(),
                        },
                    },
                    Err(e) => CallResponse::Rejected {
                        reason: e.to_string(),
                    },
                }
            }
            InboundCall::RequestAttachment {
                attachment_id,
                requested_chunks,
            } => {
                let Some(ctx) = self.host.get().and_then(std::sync::Weak::upgrade) else {
                    return CallResponse::Rejected {
                        reason: "no host".into(),
                    };
                };
                crate::host::adapters::files::ops::serve(
                    &ctx.files,
                    attachment_id,
                    &requested_chunks,
                )
                .map_or(CallResponse::Ack, CallResponse::Ok)
            }
            InboundCall::CheckpointStateRequest { request } => {
                // A member's state snapshot request (plan E3.2 S7c.3): the
                // Host admits and rate-limits the signer; a refusal is a NAK.
                let Some(ctx) = self.host.get().and_then(std::sync::Weak::upgrade) else {
                    return CallResponse::Rejected {
                        reason: "no host".into(),
                    };
                };
                crate::host::adapters::governance::HostGovernance::new(ctx)
                    .answer_checkpoint_state(&request)
                    .await
                    .map_or_else(
                        || CallResponse::Rejected {
                            reason: "checkpoint state not served".into(),
                        },
                        CallResponse::Ok,
                    )
            }
            InboundCall::CheckpointHintRequest { request } => {
                // A member's or a joiner's checkpoint hint request (plan
                // E3.2 S7c.5): admitted and rate-limited like a snapshot
                // request; a refusal is a NAK.
                let Some(ctx) = self.host.get().and_then(std::sync::Weak::upgrade) else {
                    return CallResponse::Rejected {
                        reason: "no host".into(),
                    };
                };
                crate::host::adapters::governance::HostGovernance::new(ctx)
                    .answer_checkpoint_hint(&request)
                    .await
                    .map_or_else(
                        || CallResponse::Rejected {
                            reason: "checkpoint hint not served".into(),
                        },
                        CallResponse::Ok,
                    )
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

        // A community's governance record: that community is rebuilt (plan
        // E3.1 C1); otherwise an invite's inbox, a friend's participant
        // record or a friend's profile: the Host reads every changed subkey
        // (plan E2.5).
        let governance = self
            .host
            .get()
            .and_then(std::sync::Weak::upgrade)
            .map(crate::host::adapters::governance::HostGovernance::new);
        let rebuilt = match governance {
            Some(governance) => {
                governance
                    .on_watched_change(record_key, &changed_subkeys)
                    .await
            }
            None => false,
        };
        if !rebuilt {
            if let Some(host) = self.friends() {
                host.on_watched_change(record_key, &changed_subkeys, first_value.as_deref())
                    .await;
            }
        }

        // Forward to SubscriptionManager for event emission
        if let Some(ref sub_mgr) = *self.subscriptions.read() {
            sub_mgr.on_value_change(record_key, changed_subkeys, first_value);
        }
    }

    async fn on_event(&self, event: TransportEvent) {
        debug!(event = ?std::mem::discriminant(&event), "handler: on_event");
        // The friends' network-ready trigger (plan E2.5), subscribers or not.
        if let (
            Some(host),
            TransportEvent::AttachmentChanged {
                public_internet_ready,
                ..
            },
        ) = (self.friends(), &event)
        {
            host.network_changed(*public_internet_ready);
        }
        if let TransportEvent::AttachmentChanged { state, .. } = &event {
            self.sample_strength(state);
        }
        if let Some(ref sub_mgr) = *self.subscriptions.read() {
            match event {
                TransportEvent::AttachmentChanged {
                    state,
                    is_attached,
                    public_internet_ready,
                } => {
                    let (has_route, media_route) = self.route_status();
                    sub_mgr.on_attachment_change(
                        rekindle_types::subscription_events::NetworkEvent::AttachmentChanged {
                            attachment_state: state,
                            is_attached,
                            public_internet_ready,
                            has_route,
                            media_route,
                            can_write: self.can_write(),
                        },
                    );
                }
                TransportEvent::RoutesChanged => {
                    let Some(transport) = self
                        .transport
                        .read()
                        .as_ref()
                        .and_then(std::sync::Weak::upgrade)
                    else {
                        return;
                    };
                    let shared = transport.shared();
                    let (has_route, media_route) = self.route_status();
                    sub_mgr.on_attachment_change(
                        rekindle_types::subscription_events::NetworkEvent::AttachmentChanged {
                            attachment_state: shared.attachment_state().to_string(),
                            is_attached: shared.is_attached(),
                            public_internet_ready: shared.public_internet_ready(),
                            has_route,
                            media_route,
                            can_write: self.can_write(),
                        },
                    );
                }
                TransportEvent::LocalRoutesDied { count } => {
                    sub_mgr.on_route_change(count, vec![]);
                }
                TransportEvent::RemoteRoutesDied { peer_keys } => {
                    sub_mgr.on_route_change(0, peer_keys);
                }
            }
        }
    }
}

/// Publish the indicator's bars to every subscriber (plan E7.2).
fn publish_strength(subscriptions: &RwLock<Option<SubscriptionManager>>, bars: u8) {
    if let Some(manager) = subscriptions.read().as_ref() {
        manager.on_attachment_change(
            rekindle_types::subscription_events::NetworkEvent::Strength { bars },
        );
    }
}
