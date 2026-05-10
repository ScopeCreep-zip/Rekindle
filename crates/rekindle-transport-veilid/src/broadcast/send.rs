//! Outbound send paths — fire-and-forget and request/response.
//!
//! All methods accept opaque `&[u8]` — pre-signed, pre-framed by chat.
//! Transport does not inspect, sign, frame, or modify payload content.
//! This mirrors Veilid's own `app_message(target, message: Vec<u8>)` contract.

use std::sync::Arc;
use std::time::Duration;

use tracing::debug;
use veilid_core::{Target, VeilidAPI};

use crate::config::TransportConfig;
use crate::error::{TransportError, Result};
use super::node::build_routing_context;
use super::peer_registry::PeerTarget;

/// Report of a broadcast operation.
#[derive(Debug, Default)]
pub struct BroadcastReport {
    /// Number of peers successfully sent to.
    pub delivered: usize,
    /// Peers that failed with their error descriptions.
    pub failures: Vec<(String, String)>,
}

/// Fire-and-forget message sender (wraps `app_message`).
///
/// Every method accepts opaque bytes. Chat is responsible for signing,
/// framing, and serialization before calling these methods.
pub struct Sender {
    api: VeilidAPI,
    config: Arc<TransportConfig>,
}

impl Sender {
    pub(crate) fn new(api: VeilidAPI, config: Arc<TransportConfig>) -> Self {
        Self { api, config }
    }

    /// Send opaque bytes to a single peer via app_message.
    pub async fn send_raw(&self, target: &PeerTarget, data: &[u8]) -> Result<()> {
        let rc = build_routing_context(&self.api, &self.config.safety.text)?;
        rc.app_message(Target::RouteId(target.route_id.clone()), data.to_vec())
            .await
            .map_err(|e| TransportError::SendFailed {
                target: format!("{:?}", target.route_id),
                reason: e.to_string(),
            })?;
        debug!(len = data.len(), "raw message sent");
        Ok(())
    }

    /// Broadcast opaque bytes to a set of peers.
    pub async fn broadcast_raw(
        &self,
        targets: &[(String, PeerTarget)],
        data: &[u8],
    ) -> BroadcastReport {
        let rc = match build_routing_context(&self.api, &self.config.safety.text) {
            Ok(rc) => rc,
            Err(e) => return BroadcastReport {
                delivered: 0,
                failures: vec![("*".into(), format!("routing: {e}"))],
            },
        };

        let mut report = BroadcastReport::default();
        for (peer_key, target) in targets {
            match rc.app_message(Target::RouteId(target.route_id.clone()), data.to_vec()).await {
                Ok(()) => report.delivered += 1,
                Err(e) => report.failures.push((peer_key.clone(), e.to_string())),
            }
        }

        debug!(delivered = report.delivered, failed = report.failures.len(), "broadcast complete");
        report
    }

    /// Broadcast opaque bytes with bounded parallelism via JoinSet.
    pub async fn broadcast_raw_parallel(
        &self,
        targets: &[(String, PeerTarget)],
        data: &[u8],
        max_concurrent: usize,
    ) -> BroadcastReport {
        let rc = match build_routing_context(&self.api, &self.config.safety.text) {
            Ok(rc) => rc,
            Err(e) => return BroadcastReport {
                delivered: 0,
                failures: vec![("*".into(), format!("routing: {e}"))],
            },
        };

        let mut report = BroadcastReport::default();
        let mut join_set = tokio::task::JoinSet::new();
        let mut target_iter = targets.iter();
        let mut pending = 0usize;

        loop {
            while pending < max_concurrent {
                let Some((peer_key, target)) = target_iter.next() else { break };
                let rc = rc.clone();
                let frame = data.to_vec();
                let route_id = target.route_id.clone();
                let key = peer_key.clone();
                join_set.spawn(async move {
                    let result = rc.app_message(Target::RouteId(route_id), frame).await;
                    (key, result)
                });
                pending += 1;
            }

            if pending == 0 { break; }

            match join_set.join_next().await {
                Some(Ok((_, Ok(())))) => {
                    report.delivered += 1;
                }
                Some(Ok((key, Err(e)))) => {
                    report.failures.push((key, e.to_string()));
                }
                Some(Err(e)) => {
                    report.failures.push(("*".into(), format!("join: {e}")));
                }
                None => break,
            }
            pending -= 1;
        }

        debug!(delivered = report.delivered, failed = report.failures.len(), "parallel broadcast complete");
        report
    }

    /// Send opaque voice packet bytes to a single peer.
    pub async fn send_voice_raw(&self, target: &PeerTarget, data: &[u8]) -> Result<()> {
        let rc = build_routing_context(&self.api, &self.config.safety.voice)?;
        rc.app_message(Target::RouteId(target.route_id.clone()), data.to_vec())
            .await
            .map_err(|e| TransportError::SendFailed {
                target: format!("{:?}", target.route_id),
                reason: e.to_string(),
            })
    }

    /// Broadcast opaque voice packet bytes to multiple peers.
    pub async fn broadcast_voice_raw(
        &self,
        targets: &[PeerTarget],
        data: &[u8],
    ) -> BroadcastReport {
        let rc = match build_routing_context(&self.api, &self.config.safety.voice) {
            Ok(rc) => rc,
            Err(e) => return BroadcastReport {
                delivered: 0, failures: vec![("*".into(), format!("{e}"))],
            },
        };

        let mut report = BroadcastReport::default();
        for target in targets {
            match rc.app_message(Target::RouteId(target.route_id.clone()), data.to_vec()).await {
                Ok(()) => report.delivered += 1,
                Err(e) => report.failures.push((String::new(), e.to_string())),
            }
        }
        report
    }
}

/// Request/response RPC caller (wraps `app_call`).
///
/// Every method accepts opaque bytes. Chat is responsible for signing,
/// framing, and serialization before calling these methods.
pub struct Caller {
    api: VeilidAPI,
    config: Arc<TransportConfig>,
}

impl Caller {
    pub(crate) fn new(api: VeilidAPI, config: Arc<TransportConfig>) -> Self {
        Self { api, config }
    }

    /// Send opaque bytes and await opaque response via app_call.
    pub async fn call_raw(&self, target: &PeerTarget, data: &[u8]) -> Result<Vec<u8>> {
        let timeout = Duration::from_millis(self.config.rpc_timeout_ms);
        self.call_raw_with_timeout(target, data, timeout).await
    }

    /// Send opaque bytes with caller-specified timeout.
    pub async fn call_raw_with_timeout(
        &self,
        target: &PeerTarget,
        data: &[u8],
        timeout: Duration,
    ) -> Result<Vec<u8>> {
        let rc = build_routing_context(&self.api, &self.config.safety.rpc)?;

        #[allow(clippy::cast_possible_truncation)]
        let timeout_ms = timeout.as_millis() as u64;

        let response = tokio::time::timeout(
            timeout,
            rc.app_call(Target::RouteId(target.route_id.clone()), data.to_vec()),
        )
        .await
        .map_err(|_| TransportError::Timeout {
            operation: "app_call".into(),
            duration_ms: timeout_ms,
        })?
        .map_err(|e| TransportError::SendFailed {
            target: format!("{:?}", target.route_id),
            reason: e.to_string(),
        })?;

        debug!(response_len = response.len(), "RPC complete");
        Ok(response)
    }
}
