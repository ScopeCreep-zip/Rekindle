//! Sending through a Strand Relay (architecture §13.3 steps 1–2): rank the
//! recipient's published relays by `blake3(target ‖ relay blob)` (the
//! chiral lowest-hash pattern, Wave 7 P7.2), skip those whose breaker is
//! open (P7.3), and try each in rank order until one takes it.

use rekindle_codec::message::envelope::MessagePayload;
use rekindle_route::relay;

use crate::broadcast::node::TransportNode;
use crate::error::{Result, TransportError};

/// Send `inner_envelope_bytes` (a serialized `MessageEnvelope` addressed to
/// `target_pubkey`; the relay only forwards bytes) through a relay in the
/// published pool `relay_pool_body`.
///
/// A relay whose breaker is open is skipped until its cooldown ends; then
/// one half-open attempt is allowed, which closes the breaker on success.
///
/// # Errors
/// The pool has no usable relay, or every attempt failed.
pub async fn send_via_relay(
    node: &TransportNode,
    target_pubkey: &str,
    relay_pool_body: &[u8],
    inner_envelope_bytes: &[u8],
) -> Result<()> {
    let candidates = relay::decode_relay_pool(relay_pool_body);
    if candidates.is_empty() {
        return Err(TransportError::NoRoute {
            peer: "no usable relay routes in pool".to_string(),
        });
    }
    let payload = MessagePayload::RelayEnvelope {
        target_pubkey: target_pubkey.to_string(),
        inner_payload: inner_envelope_bytes.to_vec(),
    };
    let payload_bytes =
        serde_json::to_vec(&payload).map_err(|e| TransportError::SerializationFailed {
            reason: format!("relay payload: {e}"),
        })?;

    let mut last_error = None;
    let mut skipped_open = 0usize;
    for relay_blob in relay::rank_candidates(target_pubkey, candidates) {
        let key = relay::key_for(&relay_blob);
        if node
            .relay()
            .health(&key)
            .is_some_and(|h| h.is_circuit_open())
        {
            skipped_open += 1;
            continue;
        }
        // An invalid blob is a failure too, so a chronically broken route
        // goes into the breaker.
        let sent = match node.routes().import(&relay_blob) {
            Ok(target) => {
                node.sender()
                    .send_direct(&target, payload_bytes.clone())
                    .await
            }
            Err(e) => Err(TransportError::RouteImportFailed {
                peer: "relay".to_string(),
                reason: e.to_string(),
            }),
        };
        match sent {
            Ok(()) => {
                node.relay().record_success(key);
                return Ok(());
            }
            Err(e) => {
                node.relay().record_failure(key);
                last_error = Some(e);
            }
        }
    }
    if skipped_open > 0 {
        tracing::debug!(target = %target_pubkey, skipped_open,
            "all viable relays exhausted; some skipped due to open circuit breakers");
    }
    Err(last_error.unwrap_or_else(|| TransportError::NoRoute {
        peer: "all relay attempts failed".to_string(),
    }))
}
