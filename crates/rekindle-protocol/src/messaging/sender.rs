use std::time::Duration;

use ed25519_dalek::{Signer, SigningKey};
use veilid_core::{RoutingContext, Target};

use crate::capnp_codec;
use crate::error::ProtocolError;
use crate::messaging::envelope::MessageEnvelope;

/// Application-level timeout for `app_call` RPC requests.
///
/// Veilid's internal timeout is 10-30s. We apply a shorter 8s timeout
/// so the application can fail fast and retry with a fresh route.
const APP_CALL_TIMEOUT: Duration = Duration::from_secs(8);

/// Build and sign a `MessageEnvelope` from raw secret key bytes.
///
/// Convenience wrapper that creates a `SigningKey` from the 32-byte secret
/// and delegates to [`build_envelope`].
pub fn build_envelope_from_secret(
    secret_key_bytes: &[u8; 32],
    timestamp: u64,
    nonce: Vec<u8>,
    payload: Vec<u8>,
) -> MessageEnvelope {
    let signing_key = SigningKey::from_bytes(secret_key_bytes);
    build_envelope(&signing_key, timestamp, nonce, payload)
}

/// Build and sign a `MessageEnvelope` from raw components.
///
/// Signs (timestamp || nonce || payload) with the sender's Ed25519 key.
pub fn build_envelope(
    signing_key: &SigningKey,
    timestamp: u64,
    nonce: Vec<u8>,
    payload: Vec<u8>,
) -> MessageEnvelope {
    let sender_key = signing_key.verifying_key().to_bytes().to_vec();

    // Sign: timestamp || nonce || payload
    let mut signed_data = Vec::new();
    signed_data.extend_from_slice(&timestamp.to_le_bytes());
    signed_data.extend_from_slice(&nonce);
    signed_data.extend_from_slice(&payload);

    let signature = signing_key.sign(&signed_data);

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
/// The caller is responsible for importing the route (via `DHTManager::get_or_import_route`)
/// and passing the resolved `RouteId` here.
pub async fn send_envelope(
    routing_context: &RoutingContext,
    route_id: veilid_core::RouteId,
    envelope: &MessageEnvelope,
) -> Result<(), ProtocolError> {
    let data = capnp_codec::message::encode_envelope(envelope);

    routing_context
        .app_message(Target::RouteId(route_id), data)
        .await
        .map_err(|e| ProtocolError::SendFailed(format!("app_message: {e}")))?;

    tracing::debug!(
        sender = hex::encode(&envelope.sender_key),
        payload_len = envelope.payload.len(),
        "envelope sent via Veilid"
    );

    Ok(())
}

/// Send a request-response message (`app_call`) and wait for a reply.
///
/// The caller is responsible for importing the route (via `DHTManager::get_or_import_route`)
/// and passing the resolved `RouteId` here.
pub async fn send_call(
    routing_context: &RoutingContext,
    route_id: veilid_core::RouteId,
    envelope: &MessageEnvelope,
) -> Result<Vec<u8>, ProtocolError> {
    let data = capnp_codec::message::encode_envelope(envelope);

    let response = tokio::time::timeout(
        APP_CALL_TIMEOUT,
        routing_context.app_call(Target::RouteId(route_id), data),
    )
    .await
    .map_err(|_| ProtocolError::SendFailed("app_call: Timeout (8s application limit)".to_string()))?
    .map_err(|e| ProtocolError::SendFailed(format!("app_call: {e}")))?;

    tracing::debug!(
        sender = hex::encode(&envelope.sender_key),
        response_len = response.len(),
        "app_call response received"
    );

    Ok(response)
}
