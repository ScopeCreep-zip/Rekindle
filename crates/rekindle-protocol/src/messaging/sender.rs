use ed25519_dalek::{Signer, SigningKey};
use veilid_core::{RoutingContext, Target};

use crate::error::ProtocolError;
use rekindle_codec::capnp_codec;
use rekindle_codec::message::envelope::MessageEnvelope;
use rekindle_codec::message::signing::envelope_signing_bytes;

/// Build and sign a `MessageEnvelope` to `recipient` from raw secret key
/// bytes. See [`build_envelope`].
pub fn build_envelope_from_secret(
    secret_key_bytes: &[u8; 32],
    recipient: &[u8; 32],
    timestamp: u64,
    nonce: Vec<u8>,
    payload: Vec<u8>,
) -> MessageEnvelope {
    let signing_key = SigningKey::from_bytes(secret_key_bytes);
    build_envelope(&signing_key, recipient, timestamp, nonce, payload)
}

/// Build and sign a `MessageEnvelope` to `recipient` (their identity key).
///
/// The signature covers [`envelope_signing_bytes`], which binds the
/// recipient, so the envelope verifies only for them. A retry passes the
/// original nonce and payload with a fresh timestamp.
pub fn build_envelope(
    signing_key: &SigningKey,
    recipient: &[u8; 32],
    timestamp: u64,
    nonce: Vec<u8>,
    payload: Vec<u8>,
) -> MessageEnvelope {
    let sender_key = signing_key.verifying_key().to_bytes().to_vec();
    let signature = signing_key.sign(&envelope_signing_bytes(
        recipient, timestamp, &nonce, &payload,
    ));

    MessageEnvelope {
        sender_key,
        timestamp,
        nonce,
        payload,
        signature: signature.to_bytes().to_vec(),
    }
}

/// Send a message envelope to a peer via a pre-imported `RouteId`.
///
/// The message is already encrypted and wrapped in an envelope.
/// The caller imports the route (through `RouteImports`) and passes the
/// resolved `RouteId` here.
pub async fn send_envelope(
    routing_context: &RoutingContext,
    route_id: veilid_core::RouteId,
    envelope: &MessageEnvelope,
) -> Result<(), ProtocolError> {
    let data = capnp_codec::message::encode_envelope(envelope);

    routing_context
        .app_message(Target::RouteId(route_id), data)
        .await
        .map_err(|e| send_error("app_message", &e))?;

    tracing::debug!(
        sender = hex::encode(&envelope.sender_key),
        payload_len = envelope.payload.len(),
        "envelope sent via Veilid"
    );

    Ok(())
}

/// A send failure, typed so the caller releases a route only when the
/// failure shows it unusable (`NoConnection`, `InvalidTarget`).
fn send_error(what: &str, e: &veilid_core::VeilidAPIError) -> ProtocolError {
    match e {
        veilid_core::VeilidAPIError::NoConnection { .. }
        | veilid_core::VeilidAPIError::InvalidTarget { .. } => {
            ProtocolError::RouteUnusable(format!("{what}: {e}"))
        }
        _ => ProtocolError::SendFailed(format!("{what}: {e}")),
    }
}

/// Send a request-response message (`app_call`) and wait for a reply.
///
/// The caller is responsible for importing the route (via the process's
/// `RouteImports::get_or_import`) and passing the resolved `RouteId` here.
pub async fn send_call(
    routing_context: &RoutingContext,
    route_id: veilid_core::RouteId,
    envelope: &MessageEnvelope,
) -> Result<Vec<u8>, ProtocolError> {
    let data = capnp_codec::message::encode_envelope(envelope);

    // Bounded by Veilid's own reply timeout for the route's hop count
    // (`5 s + 0.5 s × max(0, 2·hops − 4)`, plan V21); an outer timeout would
    // only drop the call, not cancel it (plan C4.L1b).
    let response = routing_context
        .app_call(Target::RouteId(route_id), data)
        .await
        .map_err(|e| send_error("app_call", &e))?;

    tracing::debug!(
        sender = hex::encode(&envelope.sender_key),
        response_len = response.len(),
        "app_call response received"
    );

    Ok(response)
}
