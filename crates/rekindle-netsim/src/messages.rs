//! App messages and calls on the simulated network (plan C9.3). A message
//! or call queues for the pump, arriving after its latency
//! ([`crate::latency::MESSAGE`]); a call's reply travels back one more
//! latency later, who gives up after Veilid's reply timeout.
//!
//! Every send and call is unpinned, as `VeilidMessages` sends (plan RV1):
//! its safety route is a pool route of the send's hop count, floor-clamped
//! (Veilid 0.5.8 `route_select.rs:119-124`), and a call's reply route is a
//! pool route Veilid selects to match it (`rpc_processor/destination.rs`
//! `:487-520`). The fork holds the selected reply route only through its
//! assembly (`_held_reply_route`), so nothing holds it while the question is
//! out. A pool route of the default hop count (`default_route_hop_count`,
//! [`ANONYMITY_HOP_FLOOR`] in Rekindle's config) is kept and kept alive by
//! the 10 s loopback keepalive (`private_route_management.rs:7`, `:143-150`);
//! an automatic route of any other length is released by the management
//! task the first time it needs testing (`:115-137`, the
//! `else if v.is_automatic()` arm): once [`ROUTE_MIN_IDLE`] has passed since
//! it was last known valid (`route_stats.rs:144-158`). An answer that reaches
//! a released route is dropped (`route_validate.rs:22-33`), so the caller
//! times out ([`FakeMessages::reply_route_gone`]); any reply also crosses
//! the network as a message does: lost to a partition, a detached caller,
//! or the drop fault ([`Network::lost_in_transit`]).

use std::sync::Arc;
use std::time::Duration;

use rekindle_records::port::{
    check_message_len, AppCallId, AppMessagePort, PeerTarget, PortError, PortEvent,
};
use rekindle_types::config::{SafetyProfile, ANONYMITY_HOP_FLOOR};
use tokio::sync::oneshot;

use crate::latency;
use crate::loopback::ROUTE_MIN_IDLE;
use crate::network::Network;

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
}

impl FakeMessages {
    pub(crate) fn new(node: usize, net: Arc<Network>) -> Self {
        Self { node, net }
    }

    /// Whether this node's automatic pool routes fail end to end
    /// ([`crate::Fault::PoolRoutesFail`]).
    fn pool_failing(&self) -> bool {
        self.net
            .inner
            .lock()
            .faults
            .pool_failing
            .contains(&self.node)
    }

    /// The node owning `target`'s route, `None` when its owner released
    /// it, it is broken end to end ([`crate::loopback`]), `target` is dead
    /// from outside ([`crate::Fault::OutsideDead`]), or this node's pool
    /// safety routes fail ([`crate::Fault::PoolRoutesFail`]): what is sent
    /// is lost; `RouteUnusable` when this node's cache does not
    /// resolve it (Veilid's `NoConnection`, [`crate::routes`]) or the first
    /// hop of its safety route fails ([`crate::Fault::RouteLinkFailure`], a
    /// `Send` error: our side, the peer's route kept);
    /// `Send` when this node is detached.
    fn route_owner(&self, target: &PeerTarget) -> Result<Option<usize>, PortError> {
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
            || self.net.outside_dead(target.as_str())
            || self.pool_failing()
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
    /// A message to `target` over a pool safety route.
    async fn send_over(&self, target: &PeerTarget, message: Vec<u8>) -> Result<(), PortError> {
        check_message_len(&message)?;
        self.net.awake(self.node).await;
        let Some(to) = self.route_owner(target)? else {
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

    /// A call to `target` over a pool safety route, answered over a pool
    /// reply route.
    async fn call_over(
        &self,
        target: &PeerTarget,
        profile: &SafetyProfile,
        message: Vec<u8>,
    ) -> Result<Vec<u8>, PortError> {
        check_message_len(&message)?;
        self.net.awake(self.node).await;
        if let Some(slot) = self.net.inner.lock().nodes.get_mut(self.node) {
            slot.app_calls += 1;
        }
        let Some(to) = self.route_owner(target)? else {
            // Into a released route: no answer comes back.
            tokio::time::sleep(call_timeout(profile)).await;
            return Err(PortError::Timeout("app call timed out".into()));
        };
        if self.net.inner.lock().faults.hung.contains(&to) {
            // Stuck before the answer wait: nothing is sent, nothing ends it.
            return std::future::pending().await;
        }
        let stall = self.net.inner.lock().faults.call_stall.get(&to).copied();
        if let Some(stall) = stall {
            // Stuck before it is sent, then sent: the answer wait starts after.
            tokio::time::sleep(stall).await;
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
        self.net.select_pool_reply(self.node);
        let leg = || self.net.latency.draw(self.node, latency::MESSAGE);
        let (there, back) = (leg(), leg());
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
            // An answer lost past the callee's first hop
            // ([`crate::Fault::AnswerLost`]): the callee answered.
            let lost_on_the_way = {
                let inner = self.net.inner.lock();
                inner.faults.answer_lost.contains(&to)
                    || self.net.lost_in_transit(&inner, to, self.node)
            };
            tokio::time::sleep(back).await;
            if lost_on_the_way || self.reply_route_gone(profile) {
                return std::future::pending().await;
            }
            reply
        });
        if let Ok(Ok(reply)) = answered.await {
            self.net.answered(self.node);
            Ok(reply)
        } else {
            self.net.inner.lock().calls.remove(&call_id);
            Err(PortError::Timeout("app call timed out".into()))
        }
    }

    /// Whether our pool reply route is gone as the answer reaches it: the
    /// node's pool routes fail ([`crate::Fault::PoolRoutesFail`]), or the
    /// route is not of the default hop count and has been released
    /// ([`ROUTE_MIN_IDLE`] past its last validity).
    fn reply_route_gone(&self, profile: &SafetyProfile) -> bool {
        let inner = self.net.inner.lock();
        let default_length = profile.hop_count.max(ANONYMITY_HOP_FLOOR)
            == rekindle_types::config::VeilidStartupOptions::default().route_hop_count;
        inner.faults.pool_failing.contains(&self.node)
            || (!default_length
                && inner
                    .nodes
                    .get(self.node)
                    .and_then(|s| s.pool_reply_valid)
                    .is_none_or(|valid| valid.elapsed() >= ROUTE_MIN_IDLE))
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
        self.send_over(target, message).await
    }

    async fn call(
        &self,
        target: &PeerTarget,
        profile: &SafetyProfile,
        message: Vec<u8>,
    ) -> Result<Vec<u8>, PortError> {
        self.call_over(target, profile, message).await
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
