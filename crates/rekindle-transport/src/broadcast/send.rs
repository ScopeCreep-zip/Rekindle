//! Outbound send paths — fire-and-forget and request/response — over the
//! app-message and route ports (plan C9.1). The port maps each
//! [`SafetyProfile`](crate::config::SafetyProfile) to a Safe route, floor-clamped
//! to the anonymity floor.

use std::sync::Arc;

use rekindle_records::port::{AppMessagePort, PeerTarget, PortError, RoutePort};
use tracing::debug;

use crate::config::TransportConfig;
use crate::crypto::envelope::{sign_payload, Addressing};
use crate::error::{Result, TransportError};
use crate::frame::{self, TypeId};

/// A failed send as the transport reports it. An unusable route is
/// forgotten first, so the next send imports the peer's current blob
/// (plan C7.9e).
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
}

impl Sender {
    pub(crate) fn new(messages: Arc<dyn AppMessagePort>, routes: Arc<dyn RoutePort>) -> Self {
        Self { messages, routes }
    }

    /// Send an already-built 1:1 wire message (`operations::dm::wire`) to the
    /// peer whose route is `target`, over the safe route profile every 1:1
    /// message takes (`RouteContextSpec::rc_safe`, plan E2.1).
    ///
    /// # Errors
    /// The send failed; an unusable route is forgotten first.
    pub async fn send_direct(&self, target: &PeerTarget, message: Vec<u8>) -> Result<()> {
        let profile = rekindle_route::contexts::RouteContextSpec::rc_safe().safety_profile();
        self.messages
            .send(target, &profile, message)
            .await
            .map_err(|e| send_failure(self.routes.as_ref(), target, e))
    }

    /// Send a class-tagged message to a single peer.
    ///
    /// `class` selects the [`rekindle_types::config::SafetyProfile`] via
    /// [`rekindle_route::profile::profile_for_class`]. Every class —
    /// voice included — takes a Safe/anonymous safety route at the
    /// 3-hop Tor-class anonymity floor (sender hidden behind a route id,
    /// never the node identity); classes differ only in
    /// stability/sequencing. Threat model: see
    /// `docs/security/threat-model.md` Z12 (social graph leakage) and
    /// `docs/security/privacy-properties.md` § 1.2 (sender anonymity).
    /// `recipient` is the peer's identity key; the signature is bound to
    /// it and to `type_id`.
    pub async fn send_dm(
        &self,
        target: &PeerTarget,
        recipient: &[u8; 32],
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
            Addressing { recipient, type_id },
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
        self.messages
            .send(target, &profile, frame_bytes)
            .await
            .map_err(|e| send_failure(self.routes.as_ref(), target, e))?;

        debug!(
            type_id = type_id as u8,
            class = class.as_str(),
            hop_count = profile.hop_count,
            "DM sent",
        );
        Ok(())
    }
}

/// Request/response RPC caller (app calls).
pub struct AppCaller {
    messages: Arc<dyn AppMessagePort>,
    routes: Arc<dyn RoutePort>,
    config: Arc<TransportConfig>,
}

impl AppCaller {
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

    /// Send a signed RPC request and await the response.
    ///
    /// Bounded by Veilid's own reply timeout for the route's hop count
    /// (`rpc_processor::get_safety_selection_timeout`: 5 s plus 500 ms
    /// per hop above four, 6–7 s at our 3–4-hop safety routes). Not
    /// wrapped in a timeout of ours: dropping the call would not cancel
    /// it, and Veilid logs a dropped API future as an error (plan
    /// C4.L1b).
    pub async fn call(
        &self,
        target: &PeerTarget,
        recipient: &[u8; 32],
        type_id: TypeId,
        sender_secret: &[u8; 32],
        sender_public_hex: &str,
        request_payload: &[u8],
    ) -> Result<Vec<u8>> {
        // RPC paths don't go through envelope_queue's dedup (they're
        // synchronous one-shots, not retry-driven). seq=0 / correlation=None
        // is the convention for non-queued sends — receiver's
        // SeqTracker only applies on the app_message dispatch path.
        let signed = sign_payload(
            sender_secret,
            sender_public_hex,
            Addressing { recipient, type_id },
            0,
            None,
            request_payload,
        );
        let signed_bytes =
            postcard::to_stdvec(&signed).map_err(|e| TransportError::SerializationFailed {
                reason: e.to_string(),
            })?;
        let frame_bytes = frame::encode(type_id, &signed_bytes)?;

        let response = self
            .messages
            .call(target, &self.config.safety.rpc, frame_bytes)
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
    /// and await the reply, as [`Sender::send_direct`] sends one.
    ///
    /// # Errors
    /// The call failed or timed out; an unusable route is forgotten first.
    pub async fn call_direct(&self, target: &PeerTarget, message: Vec<u8>) -> Result<Vec<u8>> {
        let profile = rekindle_route::contexts::RouteContextSpec::rc_safe().safety_profile();
        self.messages
            .call(target, &profile, message)
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
    pub async fn call_community_envelope(
        &self,
        target: &PeerTarget,
        envelope_bytes: Vec<u8>,
    ) -> Result<Vec<u8>> {
        let response = self
            .messages
            .call(target, &self.config.safety.rpc, envelope_bytes)
            .await
            .map_err(|e| send_failure(self.routes.as_ref(), target, e))?;

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
    /// import happens here.
    pub async fn send_unframed_to_route(&self, route_blob: &[u8], data: Vec<u8>) -> Result<()> {
        let target =
            self.routes
                .import(route_blob)
                .map_err(|e| TransportError::RouteImportFailed {
                    peer: "route-blob".to_string(),
                    reason: e.to_string(),
                })?;
        self.messages
            .send(&target, &self.config.safety.rpc, data)
            .await
            .map_err(|e| send_failure(self.routes.as_ref(), &target, e))
    }
}
