//! Phase 15 — eager fetch of expression-asset chunks.
//!
//! Architecture §18.4 + §28.9 line 3286 — expression bytes (custom
//! emoji, stickers, soundboard sounds) ride in
//! `GovernanceEntry::ExpressionAdded` as an `AttachmentOffer`. Every
//! member eager-caches the chunks so the picker can render the
//! resolved bytes without a per-render fetch round-trip. This module
//! holds the eager-fetch loop the src-tauri side runs from
//! `set_governance_state` after a CRDT merge.
//!
//! Parameterised over `FilesDeps` — receives the per-community
//! governance-expression snapshot via `deps.governance_expressions_with_attachments`,
//! probes the local cache via `deps.with_cache`, picks online peers
//! via `deps.online_member_pseudonyms` + `deps.peer_route_blob`, and
//! sends chunk requests via `deps.app_call_peer`.

use rekindle_crypto::group::media_key::MediaEncryptionKey;
use rekindle_protocol::dht::community::envelope::{CommunityEnvelope, ControlPayload};
use uuid::Uuid;

use crate::deps::{FilesDeps, FilesEvent};
use crate::error::FilesError;
use crate::verify::verify_chunk;
use rekindle_types::attachment::AttachmentOffer;

/// For each expression in the community's governance snapshot whose
/// attachment chunks are not fully cached locally, iterate every
/// online peer until one returns enough chunks to complete the
/// bitmap. Emits `FilesEvent::ExpressionAssetReady` when an expression
/// becomes complete.
///
/// Best-effort: errors are logged via tracing and the next peer is
/// tried. The fn returns `Ok(())` regardless of how many expressions
/// were fully resolved (architecture §32 W15 — the picker re-renders
/// per-asset on each event, so partial progress is visible).
pub async fn eager_fetch_missing<D: FilesDeps>(deps: &D, community_id: &str) {
    if deps.ensure_cache_open(community_id).is_err() {
        return;
    }

    // Snapshot governance-tracked expressions with attachments + their
    // currently-cached bitmaps in one pass to avoid re-reading the
    // governance lock per iteration.
    let candidates: Vec<([u8; 16], AttachmentOffer)> = {
        let all = deps.governance_expressions_with_attachments(community_id);
        all.into_iter()
            .filter(|(eid, offer)| {
                let uuid = Uuid::from_bytes(*eid);
                let mut incomplete = false;
                let _ = deps.with_cache(community_id, &mut |cache| {
                    if let Ok(bitmap) = cache.bitmap_for(uuid, offer.chunk_count) {
                        incomplete = !bitmap.is_complete();
                    } else {
                        incomplete = true;
                    }
                    Ok(())
                });
                incomplete
            })
            .collect()
    };

    if candidates.is_empty() {
        return;
    }

    // Architecture §28.9 line 3286 — assets are eager-cached by every
    // member. Any online peer should hold the chunks. Iterate every
    // online peer per asset until one returns enough chunks to
    // complete the bitmap; this is multi-peer dispatch within a single
    // fetch run, not a backwards-compat fallback. A peer that joined
    // after the upload won't have the chunks yet, so we keep trying
    // others until we either complete or run out of peers.
    let online_peers = deps.online_member_pseudonyms(community_id);
    if online_peers.is_empty() {
        return;
    }

    for (expression_id, offer) in candidates {
        let mut became_complete = false;
        for target_pseudonym in &online_peers {
            let needed = remaining_chunks(deps, community_id, expression_id, offer.chunk_count);
            if needed.is_empty() {
                became_complete = true;
                break;
            }
            if let Err(e) = fetch_and_cache_chunks(
                deps,
                community_id,
                expression_id,
                &offer,
                &needed,
                target_pseudonym,
            )
            .await
            {
                tracing::debug!(
                    community = %community_id,
                    expression = %hex::encode(expression_id),
                    source = %target_pseudonym,
                    error = %e,
                    "eager expression fetch attempt failed; trying next peer",
                );
                continue;
            }
            // Re-check the bitmap — `RequestAttachment` may have returned
            // only a subset of the requested chunks.
            if remaining_chunks(deps, community_id, expression_id, offer.chunk_count).is_empty() {
                became_complete = true;
                break;
            }
        }

        if became_complete {
            deps.emit_event(FilesEvent::ExpressionAssetReady {
                community_id: community_id.to_string(),
                expression_id_hex: hex::encode(expression_id),
            });
        }
    }
}

/// Compute the chunk indices we still need locally. Out of band so the
/// async fetch loop can reload it after every peer attempt.
fn remaining_chunks<D: FilesDeps>(
    deps: &D,
    community_id: &str,
    expression_id: [u8; 16],
    chunk_count: u32,
) -> Vec<u32> {
    let uuid = Uuid::from_bytes(expression_id);
    let mut missing: Vec<u32> = (0..chunk_count).collect();
    let _ = deps.with_cache(community_id, &mut |cache| {
        if let Ok(bitmap) = cache.bitmap_for(uuid, chunk_count) {
            missing = bitmap.missing();
        }
        Ok(())
    });
    missing
}

/// Request a subset of an expression's chunks from a single peer,
/// verify each returned chunk against the offer's plaintext SHA-256,
/// re-encrypt under the same FEK, and insert into the cache.
/// Architecture §28.9 + §26 W26: malicious peers can serve poisoned
/// bytes wearing the right attachment_id, so verify-then-cache.
async fn fetch_and_cache_chunks<D: FilesDeps>(
    deps: &D,
    community_id: &str,
    expression_id: [u8; 16],
    offer: &AttachmentOffer,
    requested_chunks: &[u32],
    target_pseudonym: &str,
) -> Result<(), FilesError> {
    let route_blob = deps
        .peer_route_blob(community_id, target_pseudonym)
        .ok_or_else(|| FilesError::NotFound(format!("source {target_pseudonym} not online")))?;
    let requester_pseudonym = deps.my_pseudonym(community_id)?;

    // Architecture §18.4 — expressions live in governance, not
    // channels. The wire field `channel_id` is set to the empty string;
    // receivers (handle_request_attachment) probe their per-community
    // cache by attachment_id alone, so the channel context is unused
    // on the serve side.
    let payload = CommunityEnvelope::Control(ControlPayload::RequestAttachment {
        channel_id: String::new(),
        attachment_id: expression_id,
        requested_chunks: requested_chunks.to_vec(),
        requester_pseudonym,
    });
    let bytes = rekindle_protocol::capnp_envelope::encode_community_envelope(&payload)
        .map_err(|e| FilesError::Transport(format!("encode RequestAttachment: {e}")))?;
    let reply = deps.app_call_peer(&route_blob, bytes).await?;
    let envelope = rekindle_protocol::capnp_envelope::decode_community_envelope(&reply)
        .map_err(|e| FilesError::Transport(format!("decode reply: {e}")))?;
    let CommunityEnvelope::Control(ControlPayload::MultiAttachmentChunk { chunks }) = envelope
    else {
        return Err(FilesError::Transport("unexpected reply variant".into()));
    };

    // Unwrap FEK under the current community MEK. Note: this differs
    // from the channel-attachment path which uses
    // `historical_channel_mek(generation)` — for expressions, the
    // governance state holds the current-generation offer and we
    // assume the community MEK matches.
    let community_mek =
        deps.community_mek(community_id)
            .ok_or_else(|| FilesError::MekUnavailable {
                community: community_id.to_string(),
                generation: 0,
            })?;
    let raw_fek = community_mek
        .decrypt(&offer.wrapped_fek)
        .map_err(|e| FilesError::Decrypt(format!("unwrap expression FEK: {e}")))?;
    let fek_bytes: [u8; 32] = raw_fek
        .as_slice()
        .try_into()
        .map_err(|_| FilesError::Decrypt("wrapped FEK plaintext is not 32 bytes".into()))?;
    let fek = MediaEncryptionKey::from_bytes(fek_bytes, 0);

    let uuid = Uuid::from_bytes(expression_id);
    deps.with_cache_mut(community_id, &mut |cache, pinned| {
        for chunk in &chunks {
            let ControlPayload::AttachmentChunk {
                attachment_id,
                chunk_index,
                data,
                ..
            } = chunk
            else {
                continue;
            };
            if *attachment_id != expression_id {
                continue;
            }
            if *chunk_index >= offer.chunk_count {
                continue;
            }
            let Ok(plaintext) = fek.decrypt(data) else {
                tracing::warn!(
                    community = %community_id,
                    expression = %hex::encode(expression_id),
                    chunk = chunk_index,
                    "dropping expression chunk: FEK decrypt failed",
                );
                continue;
            };
            let Some(expected_hash) = offer.chunk_hashes.get(*chunk_index as usize) else {
                continue;
            };
            if let Err(e) = verify_chunk(&plaintext, expected_hash) {
                tracing::warn!(
                    community = %community_id,
                    expression = %hex::encode(expression_id),
                    chunk = chunk_index,
                    error = %e,
                    "dropping poisoned expression chunk; hash mismatch",
                );
                continue;
            }
            let stored = match fek.encrypt(&plaintext) {
                Ok(c) => c,
                Err(e) => {
                    tracing::debug!(
                        community = %community_id,
                        chunk = chunk_index,
                        error = %e,
                        "FEK re-encrypt of expression chunk failed",
                    );
                    continue;
                }
            };
            if let Err(e) = cache.insert(uuid, *chunk_index, &stored, pinned) {
                tracing::debug!(
                    community = %community_id,
                    chunk = chunk_index,
                    error = %e,
                    "cache insert during eager expression fetch failed",
                );
            }
        }
        Ok(())
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_mock::MockDeps;

    fn offer_with_id(eid: [u8; 16], chunk_count: u32) -> AttachmentOffer {
        AttachmentOffer {
            attachment_id: eid,
            filename: "expr.png".into(),
            mime_type: "image/png".into(),
            total_size: 0,
            chunk_count,
            chunk_size: 0,
            merkle_root: [0u8; 32],
            chunk_hashes: vec![[0u8; 32]; chunk_count as usize],
            wrapped_fek: Vec::new(),
            fek_mek_generation: 0,
        }
    }

    #[tokio::test]
    async fn no_expressions_is_no_op() {
        let deps = MockDeps::new("c1", "ch1");
        eager_fetch_missing(&deps, "c1").await;
        assert!(
            deps.calls.lock().events.is_empty(),
            "no events when nothing to fetch"
        );
        assert!(
            deps.calls.lock().app_calls.is_empty(),
            "no app_calls without candidates"
        );
    }

    #[tokio::test]
    async fn no_online_peers_skips_fetch() {
        let mut deps = MockDeps::new("c1", "ch1").with_mek(1, [0u8; 32]);
        // One incomplete expression but no online peers.
        deps.expressions = vec![([4u8; 16], offer_with_id([4u8; 16], 2))];
        eager_fetch_missing(&deps, "c1").await;
        assert!(deps.calls.lock().app_calls.is_empty(), "skip without peers");
        assert!(deps.calls.lock().events.is_empty(), "no events emitted");
    }

    #[tokio::test]
    async fn fully_cached_expression_is_skipped_from_candidates() {
        // Expression is already fully cached locally → no peer
        // fetch attempted. The mock starts with an empty cache, so to
        // simulate "fully cached" we insert all chunks for the
        // expression id first.
        let eid = [5u8; 16];
        let deps = MockDeps::new("c1", "ch1")
            .with_mek(1, [0u8; 32])
            .with_peer("peer-1", vec![1, 2, 3]);
        // Pre-load full bitmap into the cache.
        let uuid = Uuid::from_bytes(eid);
        {
            let mut cache = deps.cache.lock();
            cache.insert(uuid, 0, b"chunk0", &deps.pinned).unwrap();
            cache.insert(uuid, 1, b"chunk1", &deps.pinned).unwrap();
        }
        let mut deps = deps;
        deps.expressions = vec![(eid, offer_with_id(eid, 2))];
        eager_fetch_missing(&deps, "c1").await;
        assert!(
            deps.calls.lock().app_calls.is_empty(),
            "no fetch attempted for fully-cached expression"
        );
    }
}
