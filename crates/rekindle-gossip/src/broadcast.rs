//! Generic transport-agnostic gossip broadcast helper.

use std::future::Future;

use rekindle_codec::dedup::extract_dedup_key;
use rekindle_codec::envelope::{build_signed_envelope, SignedEnvelope};
use rekindle_types::error::{CommunityError, GossipError};
use serde::Serialize;

use crate::dedup::DedupCache;
use crate::mesh::fanout_degree;

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
