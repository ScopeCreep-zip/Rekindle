//! The Veilid side of the Veilid-free ports (plan C9.1): records over a
//! per-session [`RecordPool`] on a [`VeilidDht`] backend, routes over [`RouteImports`] and
//! [`OwnRoutes`], app messages over a [`RoutingContext`] per safety profile,
//! and the node's own lifecycle.

use std::sync::Arc;
use std::time::Duration;

use rekindle_records::port::{
    check_message_len, AppCallId, AppMessagePort, NodeLifecycle, PeerTarget, PortError, PortEvent,
    PortSet, RecordPort, RecordStore, RouteClass, RoutePort, RouteState,
};
use rekindle_route::own_routes::OwnRoutes;
use rekindle_types::config::SafetyProfile;
use tokio::sync::{mpsc, watch};
use veilid_core::{
    OperationId, RouteId, RoutingContext, Target, VeilidAPI, VeilidAPIError, VeilidUpdate,
};

use crate::dht::backend::VeilidDht;
use crate::dht::pool::{safety_selection, value_change, RecordCloser, RecordPool};
use crate::dht::route_imports::RouteImports;
use crate::error::ProtocolError;
use crate::node::{log_dropped, startup, veilid_update_name, NodeConfig, UPDATE_QUEUE};
use crate::own_routes::VeilidRouteAllocator;

/// A session's record pool per unlock, over the node's API.
pub struct VeilidRecords {
    api: VeilidAPI,
    profile: SafetyProfile,
    ready: tokio::sync::watch::Receiver<bool>,
    closer: Arc<RecordCloser>,
}

impl VeilidRecords {
    /// Pools over `api` with the DHT safety `profile`; `ready` follows
    /// `public_internet_ready`; `closer` is the node's.
    #[must_use]
    pub fn new(
        api: VeilidAPI,
        profile: SafetyProfile,
        ready: tokio::sync::watch::Receiver<bool>,
        closer: Arc<RecordCloser>,
    ) -> Self {
        Self {
            api,
            profile,
            ready,
            closer,
        }
    }
}

impl RecordStore for VeilidRecords {
    fn start_session(&self) -> Result<Arc<dyn RecordPort>, PortError> {
        let rc = self
            .api
            .routing_context()
            .map_err(|e| PortError::Node(format!("routing context: {e}")))?;
        // The pool's calls run on its own scope, which outlives the unlock
        // scope's shutdown so the pool can still close its records (C7.3).
        let scope = rekindle_lifecycle::SessionScope::new(
            "record pool",
            Arc::new(|task| tracing::error!(task, "record pool task panicked")),
        );
        let backend = VeilidDht::new(&rc, &self.profile)?;
        Ok(RecordPool::new(
            Arc::new(backend),
            scope,
            self.ready.clone(),
            Arc::clone(&self.closer),
        ))
    }
}

fn route_id(text: &str) -> Result<RouteId, PortError> {
    text.parse::<RouteId>()
        .map_err(|e| PortError::KeyText(format!("route id: {e}")))
}

/// Peers' routes through the node's one importer, and our own.
pub struct VeilidRoutes {
    imports: Arc<RouteImports>,
    own: Arc<OwnRoutes<VeilidRouteAllocator>>,
}

impl VeilidRoutes {
    #[must_use]
    pub fn new(imports: Arc<RouteImports>, own: Arc<OwnRoutes<VeilidRouteAllocator>>) -> Self {
        Self { imports, own }
    }
}

#[async_trait::async_trait]
impl RoutePort for VeilidRoutes {
    fn import(&self, blob: &[u8]) -> Result<PeerTarget, PortError> {
        Ok(PeerTarget::new(
            self.imports.get_or_import(blob)?.to_string(),
        ))
    }

    fn on_dead_remote(&self, dead: &[PeerTarget]) -> Vec<Vec<u8>> {
        // Every target came from `import`, so each parses.
        let ids: Vec<RouteId> = dead
            .iter()
            .filter_map(|t| route_id(t.as_str()).ok())
            .collect();
        self.imports.on_dead_remote(&ids)
    }

    fn invalidate_after_send_failure(&self, target: &PeerTarget) {
        if let Ok(id) = route_id(target.as_str()) {
            self.imports.invalidate_after_send_failure(&id);
        }
    }

    fn own_blob(&self, class: RouteClass) -> Option<Vec<u8>> {
        self.own.blob(class)
    }

    fn own_state(&self, class: RouteClass) -> watch::Receiver<RouteState> {
        self.own.state(class)
    }

    async fn available_own(&self, class: RouteClass) -> Vec<u8> {
        self.own.available(class).await
    }

    fn own_age(&self, class: RouteClass) -> Option<Duration> {
        self.own.age(class)
    }

    fn want_own(&self) {
        self.own.want_all();
    }

    fn want_own_class(&self, class: RouteClass) {
        self.own.want(class);
    }

    fn release_own_class(&self, class: RouteClass) {
        self.own.release(class);
    }

    fn on_dead_own(&self, dead: &[String]) {
        // Veilid names every route it reports by the id it issued.
        let ids: Vec<RouteId> = dead.iter().filter_map(|t| route_id(t).ok()).collect();
        self.own.on_dead(&ids);
    }

    fn renew_own(&self) {
        self.own.renew();
    }

    fn release_own(&self) {
        self.own.release_all();
    }

    async fn shutdown_own(&self) {
        self.own.shutdown().await;
    }
}

/// App messages and calls over Safe routes (never `Unsafe`: it leaks the
/// sender to the first relay), the hop count floor-clamped. Built from the
/// same `OwnRoutes` as [`VeilidRoutes`], whose live routes a pinned send
/// takes as its safety route.
pub struct VeilidMessages {
    api: VeilidAPI,
    own: Arc<OwnRoutes<VeilidRouteAllocator>>,
}

impl VeilidMessages {
    #[must_use]
    pub fn new(api: VeilidAPI, own: Arc<OwnRoutes<VeilidRouteAllocator>>) -> Self {
        Self { api, own }
    }

    fn context(&self, profile: &SafetyProfile) -> Result<RoutingContext, PortError> {
        self.with_safety(safety_selection(profile))
    }

    /// The context of a send pinned to our live route of `class`
    /// (`SafetySpec.preferred_route`, `safety.rs:273`); `Routing` while
    /// none is live, never a pool route in its place.
    fn pinned(
        &self,
        profile: &SafetyProfile,
        class: RouteClass,
    ) -> Result<RoutingContext, PortError> {
        let id = self
            .own
            .id(class)
            .ok_or_else(|| PortError::Routing(format!("our {class:?} route is not available")))?;
        let veilid_core::SafetySelection::Safe(mut spec) = safety_selection(profile) else {
            return Err(PortError::Routing("safety selection is not Safe".into()));
        };
        spec.preferred_route = Some(id);
        self.with_safety(veilid_core::SafetySelection::Safe(spec))
    }

    fn with_safety(
        &self,
        safety: veilid_core::SafetySelection,
    ) -> Result<RoutingContext, PortError> {
        self.api
            .routing_context()
            .map_err(|e| PortError::Node(format!("routing context: {e}")))?
            .with_safety(safety)
            .map_err(|e| PortError::Routing(format!("safety: {e}")))
    }
}

/// Veilid's text when a send's target route is not in its remote route
/// cache (never imported, expired or pushed out): veilid-core 0.5.8
/// `rpc_processor/destination.rs:362-371` `RPCError::network`, which
/// reaches the API as `NoConnection` with this message
/// (`rpc_processor/error.rs:49-51`, `:81`).
const REMOTE_ROUTE_UNRESOLVED: &str = "could not get remote private route";

/// Whether a send's error shows the **peer's** route unusable, so its blob
/// is forgotten and the peer's current one fetched (plan C7.9e). Only an
/// unresolved remote route is the peer's side. Every other `NoConnection`
/// and `InvalidTarget` is **ours**, and the peer's route stays: a send
/// failing at our safety route's first hop answers `NoConnection` and only
/// recompiles our route (`rpc_processor/mod.rs:2126-2145`,
/// `route_stats.rs:31-35`); "no routing domain" and missing own peer info
/// are our node's state (`:2071-2079`, `:1135-1153`); `InvalidTarget` is a
/// local `ServiceUnavailable` such as our node info not yet published
/// (`types/error.rs:313`, `destination.rs:456-460`) or our released route
/// (plan VU5, synthesis #9).
#[must_use]
pub fn peer_route_unusable(e: &VeilidAPIError) -> bool {
    matches!(e, VeilidAPIError::NoConnection { message } if message == REMOTE_ROUTE_UNRESOLVED)
}

/// A send's Veilid error as the port reports it: `RouteUnusable` only for
/// the peer's side ([`peer_route_unusable`]); a failure on our side is a
/// send error that keeps the peer's route.
fn send_error(e: &VeilidAPIError) -> PortError {
    if peer_route_unusable(e) {
        PortError::RouteUnusable(e.to_string())
    } else {
        PortError::Send(e.to_string())
    }
}

#[async_trait::async_trait]
impl AppMessagePort for VeilidMessages {
    async fn send(
        &self,
        target: &PeerTarget,
        profile: &SafetyProfile,
        message: Vec<u8>,
    ) -> Result<(), PortError> {
        check_message_len(&message)?;
        self.context(profile)?
            .app_message(Target::RouteId(route_id(target.as_str())?), message)
            .await
            .map_err(|e| send_error(&e))
    }

    async fn call(
        &self,
        target: &PeerTarget,
        profile: &SafetyProfile,
        message: Vec<u8>,
    ) -> Result<Vec<u8>, PortError> {
        check_message_len(&message)?;
        self.context(profile)?
            .app_call(Target::RouteId(route_id(target.as_str())?), message)
            .await
            .map_err(|e| send_error(&e))
    }

    async fn send_pinned(
        &self,
        target: &PeerTarget,
        profile: &SafetyProfile,
        class: RouteClass,
        message: Vec<u8>,
    ) -> Result<(), PortError> {
        check_message_len(&message)?;
        self.pinned(profile, class)?
            .app_message(Target::RouteId(route_id(target.as_str())?), message)
            .await
            .map_err(|e| send_error(&e))
    }

    async fn call_pinned(
        &self,
        target: &PeerTarget,
        profile: &SafetyProfile,
        class: RouteClass,
        message: Vec<u8>,
    ) -> Result<Vec<u8>, PortError> {
        check_message_len(&message)?;
        self.pinned(profile, class)?
            .app_call(Target::RouteId(route_id(target.as_str())?), message)
            .await
            .map_err(|e| send_error(&e))
    }

    async fn reply(&self, call_id: AppCallId, message: Vec<u8>) -> Result<(), PortError> {
        self.api
            .app_call_reply(OperationId::new(call_id.0), message)
            .await
            .map_err(|e| PortError::Send(format!("app call reply: {e}")))
    }
}

/// The node this process started: detach and stop it.
pub struct VeilidLifecycle {
    api: VeilidAPI,
}

impl VeilidLifecycle {
    #[must_use]
    pub fn new(api: VeilidAPI) -> Self {
        Self { api }
    }
}

#[async_trait::async_trait]
impl NodeLifecycle for VeilidLifecycle {
    async fn detach(&self) -> Result<(), PortError> {
        detach_result(self.api.detach().await)
    }

    async fn shutdown(&self) {
        self.api.clone().shutdown().await;
    }
}

/// A detach's answer as the port reports it. Veilid 0.5.8 refuses a detach
/// of a node already `Detached` with a generic "Not attached"
/// (`attachment_manager/mod.rs:527-528`); the node is where the caller
/// wants it, so that is success.
fn detach_result(answer: veilid_core::VeilidAPIResult<()>) -> Result<(), PortError> {
    match answer {
        Ok(()) => Ok(()),
        Err(VeilidAPIError::Generic { message }) if message == NOT_ATTACHED => Ok(()),
        Err(e) => Err(PortError::Node(format!("detach: {e}"))),
    }
}

/// Veilid's refusal of a detach of a detached node
/// (`attachment_manager/mod.rs:528`).
const NOT_ATTACHED: &str = "Not attached";

/// A Veilid update as the transport dispatches it, or `None` for one it
/// does not act on.
#[must_use]
pub fn to_port_event(update: VeilidUpdate) -> Option<PortEvent> {
    Some(match update {
        VeilidUpdate::AppMessage(msg) => PortEvent::AppMessage {
            message: msg.message().to_vec(),
        },
        VeilidUpdate::AppCall(call) => PortEvent::AppCall {
            call_id: AppCallId(call.id().as_u64()),
            message: call.message().to_vec(),
        },
        VeilidUpdate::ValueChange(change) => PortEvent::ValueChange(value_change(&change)),
        VeilidUpdate::RouteChange(change) => PortEvent::RouteChange {
            dead: change.dead_routes.iter().map(ToString::to_string).collect(),
            dead_remote: change
                .dead_remote_routes
                .iter()
                .map(ToString::to_string)
                .collect(),
        },
        VeilidUpdate::Attachment(attachment) => PortEvent::Attachment {
            state: attachment.state.to_string(),
            is_attached: attachment.state.is_attached(),
            public_internet_ready: attachment.public_internet_ready,
            reliable_peers: attachment.reliable_peer_count.as_u64(),
            live_peers: attachment.live_peer_count.as_u64(),
            network_size: attachment.estimated_network_size.as_u64(),
            median_latency_us: attachment
                .median_latency
                .map(veilid_core::TimestampDuration::as_u64)
                .unwrap_or_default(),
        },
        VeilidUpdate::Shutdown => PortEvent::Shutdown,
        _ => return None,
    })
}

/// A node this process starts, as ports.
pub struct VeilidPorts;

impl VeilidPorts {
    /// Start Veilid (the one startup path, shared with
    /// `RekindleNode::start`) and build its ports: record sessions under the
    /// DHT safety profile `dht`, routes and route allocation gated on
    /// `ready` (`public_internet_ready`), messages, and the node's own
    /// lifecycle. Updates arrive mapped, on a bounded channel; a full
    /// channel drops the event rather than block the core thread.
    ///
    /// # Errors
    /// Veilid failed to start or attach.
    pub async fn start(
        config: &NodeConfig,
        dht: SafetyProfile,
        ready: watch::Receiver<bool>,
    ) -> Result<(PortSet, mpsc::Receiver<PortEvent>), ProtocolError> {
        let (event_tx, event_rx) = mpsc::channel::<PortEvent>(UPDATE_QUEUE);
        let api = startup(
            config,
            Arc::new(move |update| {
                let label = veilid_update_name(&update);
                if let Some(event) = to_port_event(update) {
                    if event_tx.try_send(event).is_err() {
                        log_dropped(label);
                    }
                }
            }),
        )
        .await?;
        let own = OwnRoutes::new(VeilidRouteAllocator::new(api.clone()), ready.clone());
        let ports = PortSet {
            records: Arc::new(VeilidRecords::new(
                api.clone(),
                dht,
                ready,
                RecordCloser::new(),
            )),
            routes: Arc::new(VeilidRoutes::new(
                Arc::new(RouteImports::new(api.clone())),
                Arc::clone(&own),
            )),
            messages: Arc::new(VeilidMessages::new(api.clone(), own)),
            lifecycle: Some(Arc::new(VeilidLifecycle::new(api))),
        };
        Ok((ports, event_rx))
    }
}

#[cfg(test)]
mod tests {
    use rekindle_types::notification::AttachmentState;

    /// Only the peer's unresolved route makes it `RouteUnusable`; a
    /// `NoConnection` or `InvalidTarget` on our side keeps the peer's route
    /// (plan VU5, synthesis #9).
    #[test]
    fn only_the_peers_unresolved_route_is_route_unusable() {
        use rekindle_records::port::PortError;
        use veilid_core::VeilidAPIError;

        let peer = VeilidAPIError::no_connection(super::REMOTE_ROUTE_UNRESOLVED);
        assert!(super::peer_route_unusable(&peer));
        assert!(matches!(
            super::send_error(&peer),
            PortError::RouteUnusable(_)
        ));
        for ours in [
            VeilidAPIError::no_connection("no routing domain"),
            VeilidAPIError::no_connection("No peer info for stub private route"),
            VeilidAPIError::invalid_target("Own node info must be published to use private route"),
            VeilidAPIError::timeout(),
        ] {
            assert!(!super::peer_route_unusable(&ours), "{ours}");
            assert!(
                matches!(super::send_error(&ours), PortError::Send(_)),
                "{ours}"
            );
        }
    }

    /// Every attached state veilid-core can emit maps to an attached state
    /// on our side, through the string [`super::to_port_event`] carries.
    ///
    /// `from_veilid_string` fails closed to `Detached`, so an upstream
    /// rename or a new variant does not break the build — it silently
    /// reports a healthy node as detached. veilid-core 0.5.4 did both at
    /// once (added `attached_fair`, renamed `FullyAttached` ->
    /// `AttachedFull`) and the changelog mentioned neither. This walks
    /// upstream's own enum so the next one fails here instead. It sits
    /// beside the mapping, not beside the enum in `rekindle-types`: Tier 1
    /// must not depend on veilid-core (rule B2).
    #[test]
    fn every_veilid_attachment_string_is_understood() {
        use veilid_core::AttachmentState as Upstream;

        for upstream in [
            Upstream::Detached,
            Upstream::Attaching,
            Upstream::AttachedWeak,
            Upstream::AttachedFair,
            Upstream::AttachedGood,
            Upstream::AttachedStrong,
            Upstream::AttachedFull,
            Upstream::Detaching,
        ] {
            let rendered = upstream.to_string();
            let ours = AttachmentState::from_veilid_string(&rendered);

            assert_eq!(
                ours.is_attached(),
                upstream.is_attached(),
                "{rendered:?} parsed to {ours:?}, whose is_attached() \
                 disagrees with veilid-core"
            );

            if !matches!(upstream, Upstream::Detached) {
                assert_ne!(
                    ours,
                    AttachmentState::Detached,
                    "{rendered:?} fell through to the fail-closed Detached \
                     arm — add it to from_veilid_string"
                );
            }
        }
    }

    /// A detach of a node already detached is success; any other refusal is
    /// the port's error (plan VU4, synthesis #12).
    #[test]
    fn not_attached_is_a_successful_detach() {
        use veilid_core::VeilidAPIError;
        assert!(super::detach_result(Ok(())).is_ok());
        assert!(super::detach_result(Err(VeilidAPIError::generic("Not attached"))).is_ok());
        assert!(super::detach_result(Err(VeilidAPIError::not_initialized())).is_err());
        assert!(super::detach_result(Err(VeilidAPIError::generic("other"))).is_err());
    }
}
