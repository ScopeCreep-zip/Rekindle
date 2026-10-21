//! App messages and calls on the simulated network (plan C9.3). A message
//! or call queues for the pump, arriving after its latency
//! ([`crate::latency::MESSAGE`]); a call's reply returns directly to the
//! caller one more latency later, who gives up after Veilid's reply timeout.

use std::sync::Arc;
use std::time::Duration;

use rekindle_records::port::{
    check_message_len, AppCallId, AppMessagePort, PeerTarget, PortError, PortEvent,
};
use rekindle_types::config::SafetyProfile;
use tokio::sync::oneshot;

use crate::latency;
use crate::network::Network;

/// Veilid's reply timeout at our 3–4-hop safety routes
/// (`rpc_processor::get_safety_selection_timeout`: 6–7 s).
pub const CALL_TIMEOUT: Duration = Duration::from_secs(7);

/// A node's app messages and calls.
pub struct FakeMessages {
    node: usize,
    net: Arc<Network>,
}

impl FakeMessages {
    pub(crate) fn new(node: usize, net: Arc<Network>) -> Self {
        Self { node, net }
    }

    /// The node owning `target`'s route, or `RouteUnusable`.
    fn route_owner(&self, target: &PeerTarget) -> Result<usize, PortError> {
        self.net
            .inner
            .lock()
            .routes
            .get(target.as_str())
            .map(|r| r.node)
            .ok_or_else(|| PortError::RouteUnusable(format!("route {} is gone", target.as_str())))
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
        check_message_len(&message)?;
        let to = self.route_owner(target)?;
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
        Ok(())
    }

    async fn call(
        &self,
        target: &PeerTarget,
        _profile: &SafetyProfile,
        message: Vec<u8>,
    ) -> Result<Vec<u8>, PortError> {
        check_message_len(&message)?;
        let to = self.route_owner(target)?;
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
        // The reply travels back over the caller's route: `back` after the
        // callee answers, all within Veilid's reply timeout.
        let answered = tokio::time::timeout(CALL_TIMEOUT, async {
            let reply = rx.await;
            tokio::time::sleep(back).await;
            reply
        });
        if let Ok(Ok(reply)) = answered.await {
            Ok(reply)
        } else {
            self.net.inner.lock().calls.remove(&call_id);
            Err(PortError::Send("app call timed out".into()))
        }
    }

    async fn reply(&self, call_id: AppCallId, message: Vec<u8>) -> Result<(), PortError> {
        check_message_len(&message)?;
        let tx = self.net.inner.lock().calls.remove(&call_id.0);
        match tx {
            Some(tx) => {
                let _ = tx.send(message);
                Ok(())
            }
            None => Err(PortError::Send(format!("no call {}", call_id.0))),
        }
    }
}
