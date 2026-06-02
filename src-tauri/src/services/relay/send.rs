//! Alice's side of Strand Relay (architecture §13.3 step 1-2):
//! when a direct route to a friend is unavailable, look up their
//! published relay pool, deterministically rank the entries by
//! `blake3(target_pubkey || relay_blob)` (Wave 7 P7.2 — same chiral
//! "lowest-hash responder" pattern as MEK rotator selection), skip
//! relays whose circuit breaker is open (Wave 7 P7.3), and try each
//! candidate in rank order until one succeeds.

use std::sync::Arc;

use rekindle_protocol::messaging::envelope::MessagePayload;

use rekindle_route::relay;

use crate::state::AppState;
use crate::state_helpers;

/// Decode the published relay pool body — JSON `Vec<Vec<u8>>` — and drop
/// dummy entries (leading byte `0xFF` per `pool::get_local_relay_pool`).
pub fn decode_relay_pool(pool_body: &[u8]) -> Vec<Vec<u8>> {
    let entries: Vec<Vec<u8>> = serde_json::from_slice(pool_body).unwrap_or_default();
    entries
        .into_iter()
        .filter(|blob| !blob.is_empty() && blob[0] != 0xFF)
        .collect()
}

/// Send `inner_envelope_bytes` (an opaque, target-encrypted
/// `MessageEnvelope`) to the given friend via a Strand Relay route.
///
/// Selection is deterministic per (target, candidate set) and
/// circuit-breaker-aware: relays that have failed three consecutive
/// times are skipped for `BREAKER_COOLDOWN`. After cooldown a single
/// half-open probe is allowed, which closes the breaker on success.
///
/// `inner_envelope_bytes` must already be a serialized `MessageEnvelope`
/// addressed to the ultimate recipient — the relay only forwards bytes.
pub async fn send_via_relay(
    state: &Arc<AppState>,
    target_pubkey: &str,
    relay_pool_body: &[u8],
    inner_envelope_bytes: &[u8],
) -> Result<(), String> {
    let raw_candidates = decode_relay_pool(relay_pool_body);
    if raw_candidates.is_empty() {
        return Err("no usable relay routes in pool".into());
    }
    let candidates = relay::rank_candidates(target_pubkey, raw_candidates);

    let api =
        state_helpers::veilid_api(state).ok_or_else(|| "veilid api unavailable".to_string())?;
    let routing_context = state_helpers::safe_routing_context(state)
        .ok_or_else(|| "no routing context".to_string())?;

    let payload = MessagePayload::RelayEnvelope {
        target_pubkey: target_pubkey.to_string(),
        inner_payload: inner_envelope_bytes.to_vec(),
    };
    let payload_bytes =
        serde_json::to_vec(&payload).map_err(|e| format!("serialize relay payload: {e}"))?;

    let mut last_error: Option<String> = None;
    let mut skipped_open: usize = 0;
    for relay_blob in candidates {
        let key = relay::key_for(&relay_blob);
        // Wave 7 P7.3 — skip relays in open-circuit state. A half-open
        // (cooldown elapsed) relay is allowed exactly one probe; the
        // result of THIS attempt closes or re-opens the breaker.
        let snapshot = {
            let map = state.relay_health.lock();
            relay::lookup(&map, &key)
        };
        if let Some(ref h) = snapshot {
            if h.is_circuit_open() {
                skipped_open += 1;
                continue;
            }
        }

        let Ok(route_id) = api.import_remote_private_route(relay_blob) else {
            last_error = Some("invalid relay route blob".into());
            // Treat invalid blobs as failures so chronically broken
            // routes go into the breaker.
            relay::record_failure(&mut state.relay_health.lock(), key);
            continue;
        };
        match routing_context
            .app_message(
                veilid_core::Target::RouteId(route_id),
                payload_bytes.clone(),
            )
            .await
        {
            Ok(()) => {
                relay::record_success(&mut state.relay_health.lock(), key);
                return Ok(());
            }
            Err(e) => {
                relay::record_failure(&mut state.relay_health.lock(), key);
                last_error = Some(format!("relay send failed: {e}"));
            }
        }
    }
    if skipped_open > 0 {
        tracing::debug!(
            target = %target_pubkey,
            skipped_open,
            "all viable relays exhausted; some skipped due to open circuit breakers"
        );
    }
    Err(last_error.unwrap_or_else(|| "all relay attempts failed".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_strips_dummy_entries() {
        let mut entries: Vec<Vec<u8>> = Vec::new();
        entries.push(vec![1, 2, 3]);
        let mut dummy = vec![0u8; 16];
        dummy[0] = 0xFF;
        entries.push(dummy);
        let body = serde_json::to_vec(&entries).unwrap();
        let real = decode_relay_pool(&body);
        assert_eq!(real, vec![vec![1, 2, 3]]);
    }

    #[test]
    fn decode_empty_pool_yields_empty() {
        let real = decode_relay_pool(&[]);
        assert!(real.is_empty());
    }
}
