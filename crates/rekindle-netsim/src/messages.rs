//! App messages and calls on the simulated network (plan C9.3). A message
//! or call queues for the pump, arriving after its latency
//! ([`crate::latency::MESSAGE`]); a call's reply travels back one more
//! latency later, who gives up after Veilid's reply timeout.
//!
//! The reply route (veilid `rpc_processor/destination.rs:467-508`): the
//! callee answers over the reply private route the question named, which
//! is the caller's preferred route when the call was pinned to one
//! ([`AppMessagePort::call_pinned`]); unpinned, it is an automatic pool
//! route Veilid picked untested (`route_select.rs:283`,
//! `private_route_management.rs:110-118`). So a pinned reply is lost when
//! the pinned route is gone (released or dead) by then, and either reply
//! crosses the network as any message does: lost to a partition, a
//! detached caller, or the drop fault ([`Network::lost_in_transit`]).

use std::sync::Arc;
use std::time::Duration;

use rekindle_records::port::{
    check_message_len, AppCallId, AppMessagePort, PeerTarget, PortError, PortEvent, RouteClass,
};
use rekindle_route::own_routes::OwnRoutes;
use rekindle_types::config::{SafetyProfile, ANONYMITY_HOP_FLOOR};
use tokio::sync::oneshot;

use crate::latency;
use crate::network::Network;
use crate::routes::FakeAllocator;

/// Veilid's default RPC timeout (`network.rpc.timeout_ms`,
/// `veilid_config.rs:852`).
pub const RPC_TIMEOUT: Duration = Duration::from_millis(5_000);

/// The reply deadline of an app call over `profile`'s safety route:
/// Veilid's `get_safety_selection_timeout` (`rpc_processor/mod.rs:1688-1703`)
/// is the RPC timeout plus 500 ms per hop past 4 of twice the safety hop
/// count, with the hop count floor-clamped as our transport sends it.
#[must_use]
pub fn call_timeout(profile: &SafetyProfile) -> Duration {
    let hops = u32::from(profile.hop_count.max(ANONYMITY_HOP_FLOOR));
    RPC_TIMEOUT + Duration::from_millis(500) * (hops * 2).saturating_sub(4)
}

/// What a detached node's sends answer: its RPC processor is shut down
/// (`rpc_app_message.rs:18`, `rpc_app_call.rs:18`, `:167`: `TryAgain`
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
    /// it (what is sent is lost); `RouteUnusable` when this node's cache
    /// does not resolve it (Veilid's `NoConnection`, [`crate::routes`]) or
    /// the first hop of its own route fails ([`crate::Fault::RouteLinkFailure`]);
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
        if self.net.first_hop_failed(self.node, pinned) {
            return Err(PortError::RouteUnusable(
                "no connection: the safety route's first hop failed".into(),
            ));
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
        let Some(to) = self.route_owner(target, pinned)? else {
            // Dispatched into a released route: lost, and still "sent".
            return Ok(());
        };
        // Fire and forget: a dropped message still "sent".
        let delay = self.net.latency.draw(self.node, latency::MESSAGE);
        self.net.send_after(
            self.node,
            to,
            "AppMessage",
            PortEvent::AppMessage { message },
            true,
            delay,
        );
        // The statement returns once it is on its way: the first hop
        // (`rpc_processor/mod.rs:1882-1926`).
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
        let Some(to) = self.route_owner(target, pinned)? else {
            // Into a released route: no answer comes back.
            tokio::time::sleep(call_timeout(profile)).await;
            self.net.lost_question(self.node, target.as_str(), pinned);
            return Err(PortError::Send("app call timed out".into()));
        };
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
        let there = self.net.latency.draw(self.node, latency::MESSAGE);
        let back = self.net.latency.draw(self.node, latency::MESSAGE);
        self.net.send_after(
            self.node,
            to,
            "AppCall",
            PortEvent::AppCall {
                call_id: AppCallId(call_id),
                message,
            },
            true,
            there,
        );
        // The reply travels back over the question's reply route: `back`
        // after the callee answers, all within Veilid's reply timeout; a
        // lost reply leaves the caller waiting out the timeout.
        let answered = tokio::time::timeout(call_timeout(profile), async {
            let reply = rx.await;
            if self.reply_lost(to, pinned) {
                return std::future::pending().await;
            }
            tokio::time::sleep(back).await;
            reply
        });
        if let Ok(Ok(reply)) = answered.await {
            self.net.answered(self.node);
            Ok(reply)
        } else {
            self.net.inner.lock().calls.remove(&call_id);
            self.net.lost_question(self.node, target.as_str(), pinned);
            Err(PortError::Send("app call timed out".into()))
        }
    }

    /// Whether `from`'s answer to us is lost on its reply route: the
    /// `pinned` route is no longer ours and live, or the answer is lost in
    /// transit.
    fn reply_lost(&self, from: usize, pinned: Option<&str>) -> bool {
        let inner = self.net.inner.lock();
        let route_gone =
            pinned.is_some_and(|id| inner.routes.get(id).is_none_or(|r| r.node != self.node));
        route_gone || self.net.lost_in_transit(&inner, from, self.node)
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

    async fn reply(&self, call_id: AppCallId, message: Vec<u8>) -> Result<(), PortError> {
        check_message_len(&message)?;
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
            // (`rpc_app_call.rs:162-170`, `RPCError::ignore`), which the API
            // answers `Generic` (`rpc_processor/error.rs:82`) and our port
            // `Send`.
            None => Err(PortError::Send(format!("no call {}", call_id.0))),
        }
    }
}
