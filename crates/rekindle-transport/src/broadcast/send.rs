//! Outbound send paths — fire-and-forget and request/response.
//!
//! Both [`Sender`] and [`Caller`] delegate routing context construction
//! to [`node::build_routing_context`] — single source of truth for the
//! safety-profile-to-Veilid mapping.

use std::sync::Arc;
use std::time::Duration;

use tracing::debug;
use veilid_core::{Target, VeilidAPI};

use crate::config::TransportConfig;
use crate::crypto::envelope::sign_payload;
use crate::error::{TransportError, Result};
use crate::frame::{self, TypeId};
use super::node::build_routing_context;
use super::peer_registry::PeerTarget;
use crate::payload::gossip::SignedGossipEnvelope;

/// Report of a broadcast operation.
#[derive(Debug, Default)]
pub struct BroadcastReport {
    /// Number of peers successfully sent to.
    pub delivered: usize,
    /// Peers that failed with their error descriptions.
    pub failures: Vec<(String, String)>,
}

/// Fire-and-forget message sender (wraps `app_message`).
pub struct Sender {
    api: VeilidAPI,
    config: Arc<TransportConfig>,
}

impl Sender {
    pub(crate) fn new(api: VeilidAPI, config: Arc<TransportConfig>) -> Self {
        Self { api, config }
    }

    /// Send a DM-class message to a single peer.
    pub async fn send_dm(
        &self,
        target: &PeerTarget,
        type_id: TypeId,
        sender_secret: &[u8; 32],
        sender_public_hex: &str,
        payload: &[u8],
    ) -> Result<()> {
        let signed = sign_payload(sender_secret, sender_public_hex, payload);
        let signed_bytes = postcard::to_stdvec(&signed)
            .map_err(|e| TransportError::SerializationFailed { reason: e.to_string() })?;
        let frame_bytes = frame::encode(type_id, &signed_bytes)?;

        let rc = build_routing_context(&self.api, &self.config.safety.text)?;
        rc.app_message(Target::RouteId(target.route_id.clone()), frame_bytes)
            .await
            .map_err(|e| TransportError::SendFailed {
                target: format!("{:?}", target.route_id),
                reason: e.to_string(),
            })?;

        debug!(type_id = type_id as u8, "DM sent");
        Ok(())
    }

    /// Broadcast a signed gossip envelope to a set of peers.
    pub async fn broadcast_gossip(
        &self,
        targets: &[(String, PeerTarget)],
        envelope: &SignedGossipEnvelope,
    ) -> BroadcastReport {
        let envelope_bytes = match postcard::to_stdvec(envelope) {
            Ok(b) => b,
            Err(e) => return BroadcastReport {
                delivered: 0,
                failures: vec![("*".into(), format!("serialization: {e}"))],
            },
        };

        let frame_bytes = match frame::encode(TypeId::GossipBroadcast, &envelope_bytes) {
            Ok(f) => f,
            Err(e) => return BroadcastReport {
                delivered: 0,
                failures: vec![("*".into(), format!("frame: {e}"))],
            },
        };

        let rc = match build_routing_context(&self.api, &self.config.safety.text) {
            Ok(rc) => rc,
            Err(e) => return BroadcastReport {
                delivered: 0,
                failures: vec![("*".into(), format!("routing: {e}"))],
            },
        };

        let mut report = BroadcastReport::default();
        for (peer_key, target) in targets {
            match rc.app_message(Target::RouteId(target.route_id.clone()), frame_bytes.clone()).await {
                Ok(()) => report.delivered += 1,
                Err(e) => report.failures.push((peer_key.clone(), e.to_string())),
            }
        }

        debug!(delivered = report.delivered, failed = report.failures.len(), "gossip broadcast");
        report
    }

    /// Broadcast a signed gossip envelope with bounded parallelism.
    ///
    /// Sends to up to `max_concurrent` peers simultaneously via JoinSet.
    /// Each send has its own routing context call. Failures are collected
    /// but don't abort remaining sends.
    pub async fn broadcast_gossip_parallel(
        &self,
        targets: &[(String, PeerTarget)],
        envelope: &SignedGossipEnvelope,
        max_concurrent: usize,
    ) -> BroadcastReport {
        let envelope_bytes = match postcard::to_stdvec(envelope) {
            Ok(b) => b,
            Err(e) => return BroadcastReport {
                delivered: 0,
                failures: vec![("*".into(), format!("serialization: {e}"))],
            },
        };
        let frame_bytes = match frame::encode(TypeId::GossipBroadcast, &envelope_bytes) {
            Ok(f) => f,
            Err(e) => return BroadcastReport {
                delivered: 0,
                failures: vec![("*".into(), format!("frame: {e}"))],
            },
        };
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
                let frame = frame_bytes.clone();
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
                Some(Ok((key, Ok(())))) => {
                    report.delivered += 1;
                    let _ = key;
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

        debug!(delivered = report.delivered, failed = report.failures.len(), "parallel gossip broadcast");
        report
    }

    /// Send an encrypted, signed voice packet to a single peer.
    ///
    /// The payload must already be a serialized `VoicePayload` with signature
    /// and HMAC populated by the caller. The transport layer frames and sends.
    pub async fn send_voice(&self, target: &PeerTarget, payload: &[u8]) -> Result<()> {
        let frame_bytes = frame::encode(TypeId::VoicePacket, payload)?;
        let rc = build_routing_context(&self.api, &self.config.safety.voice)?;
        rc.app_message(Target::RouteId(target.route_id.clone()), frame_bytes)
            .await
            .map_err(|e| TransportError::SendFailed {
                target: format!("{:?}", target.route_id),
                reason: e.to_string(),
            })
    }

    /// Broadcast voice to multiple peers (mesh mode).
    pub async fn broadcast_voice(&self, targets: &[PeerTarget], payload: &[u8]) -> BroadcastReport {
        let frame_bytes = match frame::encode(TypeId::VoicePacket, payload) {
            Ok(f) => f,
            Err(e) => return BroadcastReport {
                delivered: 0, failures: vec![("*".into(), format!("{e}"))],
            },
        };

        let rc = match build_routing_context(&self.api, &self.config.safety.voice) {
            Ok(rc) => rc,
            Err(e) => return BroadcastReport {
                delivered: 0, failures: vec![("*".into(), format!("{e}"))],
            },
        };

        let mut report = BroadcastReport::default();
        for target in targets {
            match rc.app_message(Target::RouteId(target.route_id.clone()), frame_bytes.clone()).await {
                Ok(()) => report.delivered += 1,
                Err(e) => report.failures.push((String::new(), e.to_string())),
            }
        }
        report
    }
}

/// Request/response RPC caller (wraps `app_call`).
pub struct Caller {
    api: VeilidAPI,
    config: Arc<TransportConfig>,
}

impl Caller {
    pub(crate) fn new(api: VeilidAPI, config: Arc<TransportConfig>) -> Self {
        Self { api, config }
    }

    /// Send a signed RPC request and await the response.
    ///
    /// Uses the default RPC timeout from config. For operations that need
    /// longer timeouts (MEK transfer through relays, bootstrap), use
    /// `call_with_timeout`.
    pub async fn call(
        &self,
        target: &PeerTarget,
        type_id: TypeId,
        sender_secret: &[u8; 32],
        sender_public_hex: &str,
        request_payload: &[u8],
    ) -> Result<Vec<u8>> {
        let timeout = Duration::from_millis(self.config.rpc_timeout_ms);
        self.call_with_timeout(target, type_id, sender_secret, sender_public_hex, request_payload, timeout).await
    }

    /// Send a signed RPC request with a caller-specified timeout.
    ///
    /// The timeout controls how long to wait for the Veilid `app_call`
    /// round-trip. Operations that route through relays (MEK transfer,
    /// bootstrap, sync) need longer timeouts than direct peer RPCs.
    pub async fn call_with_timeout(
        &self,
        target: &PeerTarget,
        type_id: TypeId,
        sender_secret: &[u8; 32],
        sender_public_hex: &str,
        request_payload: &[u8],
        timeout: Duration,
    ) -> Result<Vec<u8>> {
        let signed = sign_payload(sender_secret, sender_public_hex, request_payload);
        let signed_bytes = postcard::to_stdvec(&signed)
            .map_err(|e| TransportError::SerializationFailed { reason: e.to_string() })?;
        let frame_bytes = frame::encode(type_id, &signed_bytes)?;

        let rc = build_routing_context(&self.api, &self.config.safety.rpc)?;

        #[allow(clippy::cast_possible_truncation)]
        let timeout_ms = timeout.as_millis() as u64;

        let response = tokio::time::timeout(
            timeout,
            rc.app_call(Target::RouteId(target.route_id.clone()), frame_bytes),
        )
        .await
        .map_err(|_| TransportError::Timeout {
            operation: format!("app_call(0x{:02x})", type_id as u8),
            duration_ms: timeout_ms,
        })?
        .map_err(|e| TransportError::SendFailed {
            target: format!("{:?}", target.route_id),
            reason: e.to_string(),
        })?;

        debug!(type_id = type_id as u8, response_len = response.len(), "RPC complete");
        Ok(response)
    }
}
