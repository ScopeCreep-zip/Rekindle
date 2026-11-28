//! Outbound send paths — fire-and-forget and request/response — over the
//! app-message and route ports (plan C9.1). The port maps each
//! [`SafetyProfile`](crate::config::SafetyProfile) to a Safe route, floor-clamped
//! to the anonymity floor.

use std::sync::Arc;

use rekindle_records::port::{AppMessagePort, PeerTarget, PortError, RoutePort};
use rekindle_types::message::MessageClass;
use tracing::debug;

use crate::config::TransportConfig;
use crate::crypto::envelope::{sign_payload, Addressing};
use crate::error::{Result, TransportError};
use crate::frame::{self, TypeId};

/// A failed send as the transport reports it. An unusable peer route is
/// forgotten first, so the next send imports the peer's current blob
/// (plan C7.9e). The port reports `RouteUnusable` only for the peer's side;
/// a `NoConnection` from our own safety route arrives as `Send` and keeps
/// the peer's route (plan VU5, `rekindle-protocol` `peer_route_unusable`).
fn send_failure(routes: &dyn RoutePort, target: &PeerTarget, e: PortError) -> TransportError {
    match e {
        PortError::RouteUnusable(reason) => {
            routes.invalidate_after_send_failure(target);
            TransportError::RouteUnusable {
                target: target.as_str().to_string(),
                reason,
            }
        }
        PortError::Send(reason) => TransportError::SendFailed {
            target: target.as_str().to_string(),
            reason,
        },
        PortError::Timeout(reason) => TransportError::Timeout {
            target: target.as_str().to_string(),
            reason,
        },
        other => other.into(),
    }
}

/// Report of a broadcast operation.
#[derive(Debug, Default)]
pub struct BroadcastReport {
    /// Number of peers successfully sent to.
    pub delivered: usize,
    /// Peers that failed with their error descriptions.
    pub failures: Vec<(String, String)>,
}

/// Fire-and-forget message sender (app messages).
pub struct Sender {
    messages: Arc<dyn AppMessagePort>,
    routes: Arc<dyn RoutePort>,
    config: Arc<TransportConfig>,
}

impl Sender {
    pub(crate) fn new(
        messages: Arc<dyn AppMessagePort>,
        routes: Arc<dyn RoutePort>,
        config: Arc<TransportConfig>,
    ) -> Self {
        Self {
            messages,
            routes,
            config,
        }
    }

    /// Send an already-built 1:1 wire message (`operations::dm::wire`) to the
    /// peer whose route is `target`, under the control profile every 1:1
    /// message takes ([`MessageClass::Text`], Reliable / PreferOrdered, plan
    /// VU decision 2), over a tested pool route Veilid picks
    /// ([`AppMessagePort::send`], plan RV1).
    /// A statement lost on the way is not charged to the route; the
    /// outbox's acks and retries and the DM chain recover it.
    ///
    /// # Errors
    /// The send failed; an unusable route is forgotten first.
    pub async fn send_direct(&self, target: &PeerTarget, message: Vec<u8>) -> Result<()> {
        let profile = self.config.safety.for_class(MessageClass::Text);
        // The route id is scrubbed to a per-run tag at the log sink
        // (`rekindle_utils::log_scrub`).
        debug!(
            route = target.as_str(),
            hops = profile.hop_count,
            len = message.len(),
            "1:1 app_message sent"
        );
        self.messages
            .send(target, profile, message)
            .await
            .map_err(|e| send_failure(self.routes.as_ref(), target, e))
    }
}

/// Request/response RPC caller (app calls).
pub struct AppCaller {
    messages: Arc<dyn AppMessagePort>,
    routes: Arc<dyn RoutePort>,
    config: Arc<TransportConfig>,
    /// The peer routes known dead, refused at import (plan RV1g B1).
    liveness: Arc<crate::liveness::Liveness>,
}

impl AppCaller {
    pub(crate) fn new(
        messages: Arc<dyn AppMessagePort>,
        routes: Arc<dyn RoutePort>,
        config: Arc<TransportConfig>,
        liveness: Arc<crate::liveness::Liveness>,
    ) -> Self {
        Self {
            messages,
            routes,
            config,
            liveness,
        }
    }

    /// Send a signed RPC request and await the response.
    ///
    /// Veilid bounds only the wait for the answer
    /// ([`crate::broadcast::deadline::answer_wait`], Veilid's
    /// `routed_timeout`: 5 s plus 10% of it per effective hop above four,
    /// 6–7 s at our 3–4-hop safety routes);
    /// the node's port bounds the whole call by our own deadline
    /// ([`crate::broadcast::deadline`], finding 78), as every call here.
    pub async fn call(
        &self,
        target: &PeerTarget,
        recipient: &[u8; 32],
        type_id: TypeId,
        sender_secret: &[u8; 32],
        sender_public_hex: &str,
        request_payload: &[u8],
        now_ms: u64,
    ) -> Result<Vec<u8>> {
        let signed = sign_payload(
            sender_secret,
            sender_public_hex,
            Addressing { recipient, type_id },
            request_payload,
            now_ms,
        );
        let signed_bytes =
            postcard::to_stdvec(&signed).map_err(|e| TransportError::SerializationFailed {
                reason: e.to_string(),
            })?;
        let frame_bytes = frame::encode(type_id, &signed_bytes)?;

        let response = self
            .messages
            .call(
                target,
                self.config.safety.for_class(MessageClass::Rpc),
                frame_bytes,
            )
            .await
            .map_err(|e| send_failure(self.routes.as_ref(), target, e))?;

        debug!(
            type_id = type_id as u8,
            response_len = response.len(),
            "RPC complete"
        );
        Ok(response)
    }

    /// `app_call` an already-built 1:1 wire message (`operations::dm::wire`)
    /// and await the reply, under the same control profile as
    /// [`Sender::send_direct`] ([`MessageClass::Text`]): Veilid
    /// picks the safety route from its pool and the reply route to match it
    /// (0.5.8 `rpc_processor/destination.rs:487-520`; plan RV1).
    ///
    /// # Errors
    /// The call failed or timed out; an unusable route is forgotten first.
    pub async fn call_direct(&self, target: &PeerTarget, message: Vec<u8>) -> Result<Vec<u8>> {
        let profile = self.config.safety.for_class(MessageClass::Text);
        debug!(
            route = target.as_str(),
            hops = profile.hop_count,
            len = message.len(),
            "1:1 app_call sent"
        );
        self.messages
            .call(target, profile, message)
            .await
            .map_err(|e| send_failure(self.routes.as_ref(), target, e))
    }

    /// `app_call` a bare, unframed Cap'n Proto `CommunityEnvelope`.
    ///
    /// This is the one send path that deliberately skips
    /// [`frame::encode`] and [`sign_payload`], because it is the format
    /// the desktop track already speaks: `services/veilid/network.rs`
    /// reads `call.message()` and hands it straight to
    /// `try_decode_community_envelope`. A framed, signed payload is
    /// unintelligible there, so a daemon rotator using `call` could
    /// never deliver a MEK to a desktop member.
    ///
    /// **Why dropping the signature is safe *here specifically*.** The
    /// only payload sent this way is a wrapped MEK, and
    /// `rekindle_secrets::mek::unwrap_mek` takes the sender's pseudonym
    /// public key as an ECDH input. Decryption therefore succeeds only
    /// if the ciphertext was produced by the holder of that sender's
    /// private key — the `sender_pseudonym` field is cryptographically
    /// bound, not asserted. An envelope-layer signature would be
    /// redundant with a property the payload already has.
    ///
    /// That reasoning does **not** generalise. Every other RPC carries
    /// its authority in plaintext fields and must keep going through
    /// `call`, which signs and verifies.
    ///
    /// `route_blob` is the blob `target` was imported from: the send and a
    /// timeout each log a `route_trace` line naming that route instance
    /// (live finding AB1).
    pub async fn call_community_envelope(
        &self,
        target: &PeerTarget,
        route_blob: &[u8],
        envelope_bytes: Vec<u8>,
    ) -> Result<Vec<u8>> {
        let profile = self.config.safety.for_class(MessageClass::Rpc);
        crate::route_trace::envelope_call_sent(target, route_blob, profile.hop_count);
        let response = self
            .messages
            .call(target, profile, envelope_bytes)
            .await
            .map_err(|e| {
                if let PortError::Timeout(reason) = &e {
                    crate::route_trace::envelope_call_timed_out(
                        target,
                        route_blob,
                        profile.hop_count,
                        reason,
                    );
                }
                send_failure(self.routes.as_ref(), target, e)
            })?;

        debug!(
            response_len = response.len(),
            "community-envelope RPC complete"
        );
        Ok(response)
    }

    /// Fire-and-forget an **unframed** payload to a peer named by their
    /// route blob.
    ///
    /// This is the gossip send. Gossip is deliberately unframed: the
    /// desktop's mesh broadcast puts a Cap'n Proto `SignedEnvelope`
    /// straight onto `app_message`, and a daemon that wrapped the same
    /// bytes in a `TypeId::GossipBroadcast` frame produced something no
    /// desktop peer could read. The envelope carries its own Ed25519
    /// signature over `(community_id, sender_pseudonym, envelope_bytes)`,
    /// so the frame's authentication would be redundant with a property
    /// the payload already has — the same argument that justifies
    /// `call_community_envelope` above.
    ///
    /// Takes the route blob rather than an imported `PeerTarget`; the
    /// import happens here. Over a pool route Veilid picks
    /// ([`AppMessagePort::send`]); a statement names no reply route.
    ///
    /// # Errors
    /// The blob is a peer route known dead (`RouteUnusable`, plan RV1g B1),
    /// or the import or the send failed.
    pub async fn send_unframed_to_route(&self, route_blob: &[u8], data: Vec<u8>) -> Result<()> {
        let target = crate::broadcast::node::routes::import_live(
            self.routes.as_ref(),
            &self.liveness,
            route_blob,
        )?;
        self.messages
            .send(
                &target,
                self.config.safety.for_class(MessageClass::Rpc),
                data,
            )
            .await
            .map_err(|e| send_failure(self.routes.as_ref(), &target, e))
    }
}
