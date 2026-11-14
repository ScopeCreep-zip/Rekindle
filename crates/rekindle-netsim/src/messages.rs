//! App messages and calls on the simulated network (plan C9.3). A message
//! or call queues for the pump, arriving after its latency
//! ([`crate::latency::MESSAGE`]); a call's reply travels back one more
//! latency later, who gives up after Veilid's reply timeout.
//!
//! The reply route (Veilid 0.5.8 `rpc_processor/destination.rs:486-528`):
//! the callee answers over the reply private route the question named,
//! which is the caller's preferred route when the call was pinned to one
//! ([`AppMessagePort::call_pinned`]); unpinned, it is an automatic pool
//! route of our 3-hop length (`route_select.rs:131-136`). The fork holds
//! the selected route only through its assembly (`destination.rs:493-525`,
//! `_held_reply_route`), so nothing holds it while the question is out.
//! An automatic route whose length is neither default (1 hop, 2 unsafe,
//! `route_spec_store/mod.rs:135-142`) is released by the management task
//! the first time it needs testing (`private_route_management.rs:115-137`,
//! the `else if v.is_automatic()` arm): once [`ROUTE_MIN_IDLE`] has passed
//! since it was last known valid (`route_stats.rs:144-158`; an answer over
//! it makes it valid, `:489-508`, a question sent over a route does not,
//! `:511-517`). An answer that reaches a released route finds no route to
//! validate (`route_validate.rs:22-33`, `rpc_route.rs:226-239`) and is
//! dropped, so the caller times out. The pinned Sending route is published,
//! so it is tested rather than released. So an unpinned reply is lost when
//! its pool route crosses that mark mid-call ([`FakeMessages::reply_route_gone`]),
//! a pinned reply when the pinned route is gone (released, dead) or broken
//! ([`crate::loopback`]) by then, and either reply crosses the network as
//! any message does: lost to a partition, a detached caller, or the drop
//! fault ([`Network::lost_in_transit`]).

use std::sync::Arc;
use std::time::Duration;

use rekindle_records::port::{
    check_message_len, AppCallId, AppMessagePort, PeerTarget, PortError, PortEvent, RouteClass,
};
use rekindle_route::own_routes::OwnRoutes;
use rekindle_types::config::{SafetyProfile, ANONYMITY_HOP_FLOOR};
use tokio::sync::oneshot;

use crate::latency;
use crate::loopback::ROUTE_MIN_IDLE;
use crate::network::Network;
use crate::routes::FakeAllocator;

/// Veilid's default RPC timeout (`network.rpc.timeout_ms`,
/// Veilid 0.5.8 `veilid_config.rs:944`).
pub const RPC_TIMEOUT: Duration = Duration::from_millis(5_000);

/// The reply deadline of an app call over `profile`'s safety route:
/// Veilid 0.5.8's `get_safety_selection_timeout`
/// (`rpc_processor/mod.rs:1985-1994`) is `routed_timeout`
/// (`:107-117`): the RPC timeout plus 10% of it (500 ms) per hop past 4 of
/// twice the safety hop count, with the hop count floor-clamped as our
/// transport sends it.
#[must_use]
pub fn call_timeout(profile: &SafetyProfile) -> Duration {
    let hops = u32::from(profile.hop_count.max(ANONYMITY_HOP_FLOOR));
    RPC_TIMEOUT + Duration::from_millis(500) * (hops * 2).saturating_sub(4)
}

/// What a detached node's sends answer: its RPC processor is shut down
/// (Veilid 0.5.8 `rpc_app_message.rs:18`, `rpc_app_call.rs:18`, `:165`: `TryAgain`
/// "not started up"), which our Veilid port maps to `Send`
/// (`rekindle-protocol` `ports.rs` `send_error`).
fn not_started() -> PortError {
    PortError::Send("try again: not started up".into())
}

/// A node's app messages and calls.
pub struct FakeMessages {
    node: usize,
    net: Arc<Network>,
    /// The node's own routes, the same `FakeRoutes` holds.
    own: Arc<OwnRoutes<FakeAllocator>>,
}

impl FakeMessages {
    pub(crate) fn new(node: usize, net: Arc<Network>, own: Arc<OwnRoutes<FakeAllocator>>) -> Self {
        Self { node, net, own }
    }

    /// Our live route of `class`, which a pinned send goes out on;
    /// `Routing` while none is live, as `VeilidMessages` answers.
    fn pinned(&self, class: RouteClass) -> Result<String, PortError> {
        self.own
            .id(class)
            .ok_or_else(|| PortError::Routing(format!("our {class:?} route is not available")))
    }

    /// The node owning `target`'s route, `None` when its owner released
    /// it or either route is broken end to end ([`crate::loopback`]: what
    /// is sent is lost); `RouteUnusable` when this node's cache does not
    /// resolve it (Veilid's `NoConnection`, [`crate::routes`]) or the first
    /// hop of its safety route fails ([`crate::Fault::RouteLinkFailure`], a
    /// `Send` error: our side, the peer's route kept);
    /// `Send` when this node is detached.
    fn route_owner(
        &self,
        target: &PeerTarget,
        pinned: Option<&str>,
    ) -> Result<Option<usize>, PortError> {
        if self.net.inner.lock().faults.detached.contains(&self.node) {
            return Err(not_started());
        }
        if !self.net.resolve_route(self.node, target.as_str()) {
            return Err(PortError::RouteUnusable(format!(
                "no connection: route {} could not be resolved",
                target.as_str()
            )));
        }
        if self.net.first_hop_failed(self.node) {
            // Our side: the port keeps the peer's route (`rekindle-protocol`
            // `ports.rs` `peer_route_unusable`, plan VU5).
            return Err(PortError::Send(
                "no connection: the safety route's first hop failed".into(),
            ));
        }
        if self.net.route_broken(target.as_str())
            || pinned.is_some_and(|id| self.net.route_broken(id))
        {
            return Ok(None);
        }
        Ok(self
            .net
            .inner
            .lock()
            .routes
            .get(target.as_str())
            .map(|r| r.node))
    }
}

impl FakeMessages {
    /// A message to `target`, over our `pinned` route when given.
    async fn send_over(
        &self,
        target: &PeerTarget,
        pinned: Option<&str>,
        message: Vec<u8>,
    ) -> Result<(), PortError> {
        check_message_len(&message)?;
        self.net.awake(self.node).await;
        let Some(to) = self.route_owner(target, pinned)? else {
            // Dispatched into a released route: lost, and still "sent".
            return Ok(());
        };
        if self.net.inner.lock().faults.send_hung.contains(&to) {
            // Stuck before the first hop takes it: nothing ends it.
            return std::future::pending().await;
        }
        // Fire and forget: a dropped message still "sent".
        let delay = self.net.latency.draw(self.node, latency::MESSAGE);
        self.net.send_after(
            self.node,
            to,
            "AppMessage",
            PortEvent::AppMessage {
                message,
                route: Some(target.as_str().to_string()),
            },
            true,
            delay,
        );
        // The statement returns once it is on its way: the first hop
        // (Veilid 0.5.8 `rpc_processor/mod.rs:2262-2320`).
        tokio::time::sleep(self.net.latency.draw(self.node, latency::FIRST_HOP)).await;
        Ok(())
    }

    /// A call to `target`, over our `pinned` route when given (its reply
    /// route too).
    async fn call_over(
        &self,
        target: &PeerTarget,
        profile: &SafetyProfile,
        pinned: Option<&str>,
        message: Vec<u8>,
    ) -> Result<Vec<u8>, PortError> {
        check_message_len(&message)?;
        self.net.awake(self.node).await;
        if let Some(slot) = self.net.inner.lock().nodes.get_mut(self.node) {
            slot.app_calls += 1;
        }
        let Some(to) = self.route_owner(target, pinned)? else {
            // Into a released route: no answer comes back.
            tokio::time::sleep(call_timeout(profile)).await;
            self.net.lost_question(self.node, target.as_str(), pinned);
            return Err(PortError::Send("app call timed out".into()));
        };
        if self.net.inner.lock().faults.hung.contains(&to) {
            // Stuck before the answer wait: nothing is sent, nothing ends it.
            return std::future::pending().await;
        }
        let (tx, rx) = oneshot::channel();
        let call_id = {
            let mut inner = self.net.inner.lock();
            inner.next_call += 1;
            let id = inner.next_call;
            if !inner.faults.unanswered.contains(&to) {
                inner.calls.insert(id, tx);
            }
            id
        };
        if pinned.is_none() {
            self.net.select_pool_reply(self.node);
        }
        let there = self.net.latency.draw(self.node, latency::MESSAGE);
        let back = self.net.latency.draw(self.node, latency::MESSAGE);
        self.net.send_after(
            self.node,
            to,
            "AppCall",
            PortEvent::AppCall {
                call_id: AppCallId(call_id),
                message,
                route: Some(target.as_str().to_string()),
            },
            true,
            there,
        );
        // The reply travels back over the question's reply route: `back`
        // after the callee answers, all within Veilid's reply timeout; a
        // lost reply leaves the caller waiting out the timeout.
        let answered = tokio::time::timeout(call_timeout(profile), async {
            let reply = rx.await;
            let lost_on_the_way = {
                let inner = self.net.inner.lock();
                self.net.lost_in_transit(&inner, to, self.node)
            };
            tokio::time::sleep(back).await;
            if lost_on_the_way || self.reply_route_gone(pinned) {
                return std::future::pending().await;
            }
            reply
        });
        if let Ok(Ok(reply)) = answered.await {
            self.net.answered(self.node, pinned.is_none());
            Ok(reply)
        } else {
            self.net.inner.lock().calls.remove(&call_id);
            self.net.lost_question(self.node, target.as_str(), pinned);
            Err(PortError::Send("app call timed out".into()))
        }
    }

    /// Whether our reply route is gone as the answer reaches it: the
    /// `pinned` route is no longer ours and live, or is broken; unpinned,
    /// the pool reply route has been released ([`ROUTE_MIN_IDLE`] past its
    /// last validity).
    fn reply_route_gone(&self, pinned: Option<&str>) -> bool {
        let inner = self.net.inner.lock();
        match pinned {
            Some(id) => {
                inner.routes.get(id).is_none_or(|r| r.node != self.node)
                    || inner.faults.loopback.contains_key(id)
            }
            None => inner
                .nodes
                .get(self.node)
                .and_then(|s| s.pool_reply_valid)
                .is_none_or(|valid| valid.elapsed() >= ROUTE_MIN_IDLE),
        }
    }
}

#[async_trait::async_trait]
impl AppMessagePort for FakeMessages {
    async fn send(
        &self,
        target: &PeerTarget,
        _profile: &SafetyProfile,
        message: Vec<u8>,
    ) -> Result<(), PortError> {
        self.send_over(target, None, message).await
    }

    async fn call(
        &self,
        target: &PeerTarget,
        profile: &SafetyProfile,
        message: Vec<u8>,
    ) -> Result<Vec<u8>, PortError> {
        self.call_over(target, profile, None, message).await
    }

    async fn send_pinned(
        &self,
        target: &PeerTarget,
        _profile: &SafetyProfile,
        class: RouteClass,
        message: Vec<u8>,
    ) -> Result<(), PortError> {
        let pinned = self.pinned(class)?;
        self.send_over(target, Some(&pinned), message).await
    }

    async fn call_pinned(
        &self,
        target: &PeerTarget,
        profile: &SafetyProfile,
        class: RouteClass,
        message: Vec<u8>,
    ) -> Result<Vec<u8>, PortError> {
        let pinned = self.pinned(class)?;
        self.call_over(target, profile, Some(&pinned), message)
            .await
    }

    async fn call_own(
        &self,
        class: RouteClass,
        profile: &SafetyProfile,
        message: Vec<u8>,
    ) -> Result<Vec<u8>, PortError> {
        // As `VeilidMessages`: import our own route for the call, call it
        // with itself as the safety route, and drop the import.
        let own = self.pinned(class)?;
        self.net.import_route(self.node, &own);
        let answer = self
            .call_over(&PeerTarget::new(own.clone()), profile, Some(&own), message)
            .await;
        self.net.forget_import(self.node, &own);
        answer
    }

    async fn reply(&self, call_id: AppCallId, message: Vec<u8>) -> Result<(), PortError> {
        check_message_len(&message)?;
        self.net.awake(self.node).await;
        let tx = {
            let mut inner = self.net.inner.lock();
            if inner.faults.detached.contains(&self.node) {
                return Err(not_started());
            }
            inner.calls.remove(&call_id.0)
        };
        match tx {
            Some(tx) => {
                let _ = tx.send(message);
                Ok(())
            }
            // An expired call's reply is ignored: its waiter is gone
            // (Veilid 0.5.8 `rpc_app_call.rs:162-170`, `RPCError::ignore`), which the API
            // answers `Generic` (`rpc_processor/error.rs:83`) and our port
            // `Send`.
            None => Err(PortError::Send(format!("no call {}", call_id.0))),
        }
    }
}
