//! Generic transport-agnostic gossip broadcast helper.

use std::future::Future;

use rekindle_codec::dedup::extract_dedup_key;
use rekindle_codec::envelope::{build_signed_envelope, SignedEnvelope};
use rekindle_protocol::capnp_envelope::encode_community_envelope;
use rekindle_protocol::dht::community::envelope::CommunityEnvelope;
use rekindle_types::error::{CommunityError, GossipError};
use serde::Serialize;

use crate::dedup::DedupCache;
use crate::mesh::fanout_degree;

/// Phase 20 — pure community-envelope dedup-key extractor.
///
/// Mirrors src-tauri's `extract_mesh_dedup_key`: returns a stable
/// string the dedup cache uses to gate duplicate broadcasts. Different
/// envelope variants use different bucketing strategies:
///
/// - `MessageNotification` — use the message_id directly
/// - `TypingIndicator` — 5-second buckets per channel + sender
/// - `PresenceUpdate` — 30-second buckets per sender
/// - `Control` — full 16-byte BLAKE2b hash of the encoded envelope
/// - `WatchRelay` — (record_key, subkey, content_hash) tuple
#[must_use]
pub fn extract_mesh_dedup_key(envelope: &CommunityEnvelope) -> String {
    match envelope {
        CommunityEnvelope::MessageNotification { message_id, .. } => message_id.clone(),
        CommunityEnvelope::TypingIndicator {
            channel_id,
            pseudonym_key,
        } => {
            let bucket = rekindle_utils::timestamp_secs() / 5;
            format!("typing:{channel_id}:{pseudonym_key}:{bucket}")
        }
        CommunityEnvelope::PresenceUpdate { pseudonym_key, .. } => {
            let bucket = rekindle_utils::timestamp_secs() / 30;
            format!("presence:{pseudonym_key}:{bucket}")
        }
        CommunityEnvelope::Control(_) => {
            use blake2::{digest::consts::U16, Blake2b, Digest};
            let bytes = encode_community_envelope(envelope).unwrap_or_default();
            let mut hash = Blake2b::<U16>::new();
            hash.update(&bytes);
            hex::encode(hash.finalize())
        }
        CommunityEnvelope::WatchRelay {
            record_key,
            subkey,
            content_hash,
            ..
        } => format!("watch:{record_key}:{subkey}:{content_hash}"),
    }
}

/// Build a signed envelope, insert it into the dedup cache, and broadcast
/// it to the selected fan-out using the provided transport callback.
pub async fn broadcast<T, I, F, Fut>(
    master_secret: &[u8; 32],
    community_id: &str,
    payload: &T,
    dedup_cache: &mut DedupCache,
    routes: I,
    mut send_fn: F,
) -> Result<SignedEnvelope, CommunityError>
where
    T: Serialize,
    I: IntoIterator<Item = Vec<u8>>,
    F: FnMut(Vec<u8>, Vec<u8>) -> Fut,
    Fut: Future<Output = Result<(), String>>,
{
    let signed = build_signed_envelope(master_secret, community_id, payload)?;
    let dedup_key = extract_dedup_key(&signed);
    dedup_cache.check_and_insert(community_id, &signed.sender_pseudonym, &dedup_key);

    let signed_bytes = serde_json::to_vec(&signed)?;
    let route_list: Vec<Vec<u8>> = routes.into_iter().collect();
    let degree = fanout_degree(route_list.len());
    let mut sent = 0usize;

    for route_blob in route_list.into_iter().take(degree) {
        if let Err(error) = send_fn(route_blob, signed_bytes.clone()).await {
            tracing::debug!(error = %error, "gossip send callback failed");
            continue;
        }
        sent += 1;
    }

    if degree > 0 && sent == 0 {
        return Err(CommunityError::Gossip(GossipError::BroadcastFailed(
            "all gossip sends failed".to_string(),
        )));
    }

    Ok(signed)
}
