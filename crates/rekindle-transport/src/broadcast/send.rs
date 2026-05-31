//! Outbound send paths — fire-and-forget and request/response.
//!
//! Both [`Sender`] and [`Caller`] delegate routing context construction
//! to [`node::build_routing_context`] — single source of truth for the
//! safety-profile-to-Veilid mapping.

use std::sync::Arc;
use std::time::Duration;

use tracing::debug;
use veilid_core::{Target, VeilidAPI};

use super::node::build_routing_context;
use super::peer_registry::PeerTarget;
use crate::config::TransportConfig;
use crate::crypto::envelope::sign_payload;
use crate::error::{Result, TransportError};
use crate::frame::{self, TypeId};
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

    /// Send a class-tagged message to a single peer.
    ///
    /// Phase 9 — `class` selects the [`rekindle_types::config::SafetyProfile`]
    /// via [`rekindle_route::profile::profile_for_class`]. Voice frames
    /// take the Unsafe/direct path (0-hop, sender visible — call
    /// participants are mutually known by design). All other classes
    /// take a Safe/anonymous safety route (1–2 hops, sender hidden via
    /// route ID rather than node identity). Threat model: see
    /// `docs/security/threat-model.md` Z12 (social graph leakage) and
    /// `docs/security/privacy-properties.md` § 1.2 (sender anonymity).
    ///
    /// Prior to Phase 9 every DM used `self.config.safety.text`
    /// unconditionally, which silently routed Voice through a 2-hop
    /// safety route (latency disaster) and DHT writes through the same
    /// path as plaintext DMs (acceptable but coincidental).
    pub async fn send_dm(
        &self,
        target: &PeerTarget,
        class: rekindle_types::message::MessageClass,
        type_id: TypeId,
        sender_secret: &[u8; 32],
        sender_public_hex: &str,
        seq: u64,
        correlation_id: Option<&str>,
        payload: &[u8],
    ) -> Result<()> {
        let signed = sign_payload(
            sender_secret,
            sender_public_hex,
            seq,
            correlation_id,
            payload,
        );
        let signed_bytes =
            postcard::to_stdvec(&signed).map_err(|e| TransportError::SerializationFailed {
                reason: e.to_string(),
            })?;
        let frame_bytes = frame::encode(type_id, &signed_bytes)?;

        let profile = rekindle_route::profile::profile_for_class(class);
        let rc = build_routing_context(&self.api, &profile)?;
        rc.app_message(Target::RouteId(target.route_id.clone()), frame_bytes)
            .await
            .map_err(|e| TransportError::SendFailed {
                target: format!("{:?}", target.route_id),
                reason: e.to_string(),
            })?;

        debug!(
            type_id = type_id as u8,
            class = class.as_str(),
            hop_count = profile.hop_count,
            sender_anonymous = profile.sender_anonymous,
            "DM sent",
        );
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
            Err(e) => {
                return BroadcastReport {
                    delivered: 0,
                    failures: vec![("*".into(), format!("serialization: {e}"))],
                }
            }
        };

        let frame_bytes = match frame::encode(TypeId::GossipBroadcast, &envelope_bytes) {
            Ok(f) => f,
            Err(e) => {
                return BroadcastReport {
                    delivered: 0,
                    failures: vec![("*".into(), format!("frame: {e}"))],
                }
            }
        };

        let rc = match build_routing_context(&self.api, &self.config.safety.text) {
            Ok(rc) => rc,
            Err(e) => {
                return BroadcastReport {
                    delivered: 0,
                    failures: vec![("*".into(), format!("routing: {e}"))],
                }
            }
        };

        let mut report = BroadcastReport::default();
        for (peer_key, target) in targets {
            match rc
                .app_message(
                    Target::RouteId(target.route_id.clone()),
                    frame_bytes.clone(),
                )
                .await
            {
                Ok(()) => report.delivered += 1,
                Err(e) => report.failures.push((peer_key.clone(), e.to_string())),
            }
        }

        debug!(
            delivered = report.delivered,
            failed = report.failures.len(),
            "gossip broadcast"
        );
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
            Err(e) => {
                return BroadcastReport {
                    delivered: 0,
                    failures: vec![("*".into(), format!("{e}"))],
                }
            }
        };

        let rc = match build_routing_context(&self.api, &self.config.safety.voice) {
            Ok(rc) => rc,
            Err(e) => {
                return BroadcastReport {
                    delivered: 0,
                    failures: vec![("*".into(), format!("{e}"))],
                }
            }
        };

        let mut report = BroadcastReport::default();
        for target in targets {
            match rc
                .app_message(
                    Target::RouteId(target.route_id.clone()),
                    frame_bytes.clone(),
                )
                .await
            {
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
        self.call_with_timeout(
            target,
            type_id,
            sender_secret,
            sender_public_hex,
            request_payload,
            timeout,
        )
        .await
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
        // RPC paths don't go through envelope_queue's dedup (they're
        // synchronous one-shots, not retry-driven). seq=0 / correlation=None
        // is the convention for non-queued sends — receiver's
        // SeqTracker only applies on the app_message dispatch path.
        let signed = sign_payload(sender_secret, sender_public_hex, 0, None, request_payload);
        let signed_bytes =
            postcard::to_stdvec(&signed).map_err(|e| TransportError::SerializationFailed {
                reason: e.to_string(),
            })?;
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

        debug!(
            type_id = type_id as u8,
            response_len = response.len(),
            "RPC complete"
        );
        Ok(response)
    }
}
