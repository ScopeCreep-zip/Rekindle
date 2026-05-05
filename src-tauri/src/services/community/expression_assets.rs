//! Lost Cargo storage and retrieval for expression assets (architecture
//! §18.4 + §28.9 line 3286).
//!
//! Expression bytes (custom emoji, stickers, soundboard sounds) used to
//! travel inline inside `GovernanceEntry::ExpressionAdded`, which broke
//! once any single asset exceeded Veilid's 32 KiB SMPL subkey limit (see
//! `veilid-core::storage_manager::types::encrypted_value_data::MAX_LEN`).
//! The new flow:
//!
//!   * **Upload**: chunk the bytes via `rekindle-files`, encrypt each
//!     chunk under a fresh per-asset FEK, store chunks in the per-community
//!     file cache, wrap the FEK under the current community MEK, and
//!     return an `AttachmentOffer` that the caller embeds in the
//!     `ExpressionAdded` entry.
//!   * **Download**: receivers see new `ExpressionAdded` entries during
//!     CRDT merge, look up the asset in their local cache, and (if
//!     missing) send a `RequestAttachment` `app_call` to an online peer
//!     who has eager-cached it. This module provides the eager-fetch
//!     loop that's invoked from `state_helpers::set_governance_state`.
//!   * **Render**: `services/community/expressions::list_expressions`
//!     calls `read_bytes_from_cache` to materialise the plaintext for
//!     base64-encoding to the frontend; missing assets surface as
//!     `inline_data_base64 = None` until the eager fetch completes.

use std::sync::Arc;

use rekindle_crypto::group::media_key::MediaEncryptionKey;
use rekindle_files::{verify_chunk, AttachmentOffer, Chunker, CHUNK_SIZE_BYTES};
use rekindle_protocol::dht::community::envelope::{CommunityEnvelope, ControlPayload};
use tauri::Emitter;
use uuid::Uuid;

use crate::channels::CommunityEvent;
use crate::state::{AppState, SharedState};
use crate::state_helpers;

use super::files::ensure_cache_open;

/// Chunk + encrypt + cache an expression asset, returning the
/// `AttachmentOffer` the caller embeds in `ExpressionAdded`.
///
/// `expression_id` doubles as the cache attachment_id so the chunk
/// directory layout stays stable across restarts and the eager-fetch loop
/// can probe by expression_id alone.
pub fn upload_to_cache(
    state: &SharedState,
    community_id: &str,
    expression_id: [u8; 16],
    bytes: &[u8],
    filename: String,
    mime_type: String,
) -> Result<AttachmentOffer, String> {
    let total_size = bytes.len() as u64;

    let (community_mek, mek_generation) = {
        let mek = state
            .mek_cache
            .lock()
            .get(community_id)
            .cloned()
            .ok_or_else(|| {
                "community MEK not available — wait for MEK delivery and retry".to_string()
            })?;
        let gen_value = state
            .communities
            .read()
            .get(community_id)
            .map_or(0, |c| c.mek_generation);
        (mek, gen_value)
    };

    let fek = MediaEncryptionKey::generate(0);

    ensure_cache_open(state, community_id)?;
    let chunked = Chunker::chunk(bytes).map_err(|e| format!("chunker failed: {e}"))?;
    let chunk_count = u32::try_from(chunked.chunks.len())
        .map_err(|_| "chunk count exceeds u32::MAX".to_string())?;
    let attachment_uuid = Uuid::from_bytes(expression_id);

    {
        let mut caches = state.file_caches.write();
        let cache = caches
            .get_mut(community_id)
            .ok_or_else(|| "file cache not open for community".to_string())?;
        let pinned = state
            .pinned_attachments
            .read()
            .get(community_id)
            .cloned()
            .unwrap_or_default();
        for (idx, chunk) in chunked.chunks.iter().enumerate() {
            let ciphertext = fek
                .encrypt(chunk)
                .map_err(|e| format!("FEK encrypt chunk {idx}: {e}"))?;
            let chunk_idx = u32::try_from(idx)
                .map_err(|_| "chunk index exceeds u32::MAX".to_string())?;
            cache
                .insert(attachment_uuid, chunk_idx, &ciphertext, &pinned)
                .map_err(|e| format!("cache insert chunk {chunk_idx}: {e}"))?;
        }
    }

    let wrapped_fek = community_mek
        .encrypt(fek.as_bytes())
        .map_err(|e| format!("wrap FEK: {e}"))?;

    Ok(AttachmentOffer {
        attachment_id: expression_id,
        filename,
        mime_type,
        total_size,
        chunk_count,
        chunk_size: u32::try_from(CHUNK_SIZE_BYTES).unwrap_or(u32::MAX),
        merkle_root: chunked.merkle_root,
        chunk_hashes: chunked.chunk_hashes,
        wrapped_fek,
        fek_mek_generation: mek_generation,
    })
}

/// Materialise the plaintext bytes for an expression asset from the local
/// chunk cache. Returns `None` (without erroring) when any chunk is
/// missing — the eager-fetch loop will fill the gap on the next merge.
pub fn read_bytes_from_cache(
    state: &SharedState,
    community_id: &str,
    offer: &AttachmentOffer,
) -> Option<Vec<u8>> {
    // For now, only the *current* community MEK can unwrap. A full
    // implementation would walk historical MEKs by `fek_mek_generation`;
    // expressions uploaded under an old MEK will read as missing until
    // they are re-uploaded after a rotation. Architecture §7 leaves the
    // historical-MEK store as a follow-on for cross-rotation playback.
    let community_mek = state.mek_cache.lock().get(community_id).cloned()?;
    let raw_fek = community_mek.decrypt(&offer.wrapped_fek).ok()?;
    let fek_bytes: [u8; 32] = raw_fek.as_slice().try_into().ok()?;
    let fek = MediaEncryptionKey::from_bytes(fek_bytes, 0);

    let attachment_uuid = Uuid::from_bytes(offer.attachment_id);
    let mut caches = state.file_caches.write();
    let cache = caches.get_mut(community_id)?;

    let mut plaintext = Vec::with_capacity(usize::try_from(offer.total_size).unwrap_or(0));
    for idx in 0..offer.chunk_count {
        let ciphertext = cache.get(attachment_uuid, idx).ok().flatten()?;
        let chunk = fek.decrypt(&ciphertext).ok()?;
        plaintext.extend_from_slice(&chunk);
    }
    Some(plaintext)
}

/// Diff the merged `governance_state.expressions` against the local file
/// cache and broadcast a `RequestAttachment` for any missing chunks.
/// Architecture §18.4 line 2505 + §28.9 line 3286 — eager (automatic)
/// caching with no user action.
///
/// Best-effort: silently skips when the cache root isn't initialised yet,
/// when no online peers are reachable, or when an `app_call` fails. The
/// loop fires again on every governance merge so transient failures
/// recover on the next tick.
pub async fn eager_fetch_missing(state: &Arc<AppState>, community_id: &str) {
    let Ok(()) = ensure_cache_open(state, community_id) else {
        return;
    };

    let missing: Vec<([u8; 16], AttachmentOffer)> = {
        let communities = state.communities.read();
        let Some(community) = communities.get(community_id) else {
            return;
        };
        let Some(gov) = community.governance_state.as_ref() else {
            return;
        };
        let caches = state.file_caches.read();
        let Some(cache) = caches.get(community_id) else {
            return;
        };
        gov.expressions
            .iter()
            .filter_map(|(eid, expr)| {
                let offer = expr.attachment.clone()?;
                let bitmap = cache
                    .bitmap_for(Uuid::from_bytes(*eid), offer.chunk_count)
                    .ok()?;
                if bitmap.is_complete() {
                    None
                } else {
                    Some((*eid, offer))
                }
            })
            .collect()
    };

    if missing.is_empty() {
        return;
    }

    // Architecture §28.9 line 3286 — assets are eager-cached by every
    // member. Any online peer should hold the chunks. Iterate every
    // online peer per asset until one returns enough chunks to complete
    // the bitmap; this is multi-peer dispatch within a single fetch run,
    // not a backwards-compat fallback. A peer that joined after the
    // upload won't have the chunks yet, so we keep trying others until
    // we either complete or run out of peers.
    let online_peers: Vec<String> = {
        let communities = state.communities.read();
        let Some(community) = communities.get(community_id) else {
            return;
        };
        let Some(gossip) = community.gossip.as_ref() else {
            return;
        };
        gossip.online_members.keys().cloned().collect()
    };
    if online_peers.is_empty() {
        return;
    }

    for (expression_id, offer) in missing {
        let mut became_complete = false;
        for target_pseudonym in &online_peers {
            let needed = remaining_chunks(state, community_id, expression_id, offer.chunk_count);
            if needed.is_empty() {
                became_complete = true;
                break;
            }
            if let Err(e) = fetch_and_cache_chunks(
                state,
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
            // Re-check the bitmap; `RequestAttachment` may have returned
            // only a subset of the requested chunks.
            if remaining_chunks(state, community_id, expression_id, offer.chunk_count).is_empty() {
                became_complete = true;
                break;
            }
        }

        if became_complete {
            // Architecture §32 W15 — tell the UI the picker can now
            // render the resolved bytes for this expression.
            if let Some(app_handle) = state_helpers::app_handle(state) {
                let _ = app_handle.emit(
                    "community-event",
                    CommunityEvent::ExpressionAssetReady {
                        community_id: community_id.to_string(),
                        expression_id: hex::encode(expression_id),
                    },
                );
            }
        }
    }
}

/// Compute the chunk indices we still need locally. Out of band so the
/// async fetch loop can reload it after every peer attempt.
fn remaining_chunks(
    state: &Arc<AppState>,
    community_id: &str,
    expression_id: [u8; 16],
    chunk_count: u32,
) -> Vec<u32> {
    let caches = state.file_caches.read();
    let Some(cache) = caches.get(community_id) else {
        return (0..chunk_count).collect();
    };
    match cache.bitmap_for(Uuid::from_bytes(expression_id), chunk_count) {
        Ok(bitmap) => bitmap.missing(),
        Err(_) => (0..chunk_count).collect(),
    }
}

async fn fetch_and_cache_chunks(
    state: &Arc<AppState>,
    community_id: &str,
    expression_id: [u8; 16],
    offer: &AttachmentOffer,
    requested_chunks: &[u32],
    target_pseudonym: &str,
) -> Result<(), String> {
    let route_blob = {
        let communities = state.communities.read();
        let community = communities
            .get(community_id)
            .ok_or_else(|| "community not found".to_string())?;
        let gossip = community
            .gossip
            .as_ref()
            .ok_or_else(|| "no gossip overlay".to_string())?;
        gossip
            .online_members
            .get(target_pseudonym)
            .map(|m| m.route_blob.clone())
            .ok_or_else(|| format!("source {target_pseudonym} not online"))?
    };
    let requester_pseudonym = {
        let communities = state.communities.read();
        communities
            .get(community_id)
            .and_then(|c| c.my_pseudonym_key.clone())
            .ok_or_else(|| "no pseudonym".to_string())?
    };

    // Architecture §18.4 — expressions live in governance, not channels.
    // The wire field `channel_id` is set to the empty string; receivers
    // (handle_request_attachment) probe their per-community cache by
    // attachment_id alone, so the channel context is unused on the
    // serve side.
    let payload = CommunityEnvelope::Control(ControlPayload::RequestAttachment {
        channel_id: String::new(),
        attachment_id: expression_id,
        requested_chunks: requested_chunks.to_vec(),
        requester_pseudonym,
    });
    let bytes = rekindle_protocol::capnp_envelope::encode_community_envelope(&payload)
        .map_err(|e| format!("encode RequestAttachment: {e}"))?;
    let api = state_helpers::veilid_api(state).ok_or_else(|| "Veilid API unavailable".to_string())?;
    let route_id = api
        .import_remote_private_route(route_blob)
        .map_err(|e| format!("import route: {e}"))?;
    let rc = state_helpers::safe_routing_context(state).ok_or_else(|| "not attached".to_string())?;
    let reply = rc
        .app_call(veilid_core::Target::RouteId(route_id), bytes)
        .await
        .map_err(|e| format!("app_call failed: {e}"))?;
    let envelope: CommunityEnvelope =
        rekindle_protocol::capnp_envelope::decode_community_envelope(&reply)
            .map_err(|e| format!("decode reply: {e}"))?;
    let CommunityEnvelope::Control(ControlPayload::MultiAttachmentChunk { chunks }) = envelope
    else {
        return Err("unexpected reply variant".into());
    };

    // Architecture §28.9 + §26 W26 — verify each received chunk before
    // caching: a malicious peer can serve poisoned bytes wearing the
    // right `attachment_id`, so the FEK-decrypted plaintext must hash
    // to the value the AttachmentOffer (which lives signed in the
    // governance state) committed to. Chunks that fail verify are
    // silently dropped; the caller will retry against another peer.
    let community_mek = state
        .mek_cache
        .lock()
        .get(community_id)
        .cloned()
        .ok_or_else(|| "community MEK not available — cannot verify chunks".to_string())?;
    let raw_fek = community_mek
        .decrypt(&offer.wrapped_fek)
        .map_err(|e| format!("unwrap expression FEK: {e}"))?;
    let fek_bytes: [u8; 32] = raw_fek
        .as_slice()
        .try_into()
        .map_err(|_| "wrapped FEK plaintext is not 32 bytes".to_string())?;
    let fek = MediaEncryptionKey::from_bytes(fek_bytes, 0);

    let pinned = state
        .pinned_attachments
        .read()
        .get(community_id)
        .cloned()
        .unwrap_or_default();
    let mut caches = state.file_caches.write();
    let cache = caches
        .get_mut(community_id)
        .ok_or_else(|| "cache not open".to_string())?;
    let attachment_uuid = Uuid::from_bytes(expression_id);

    for chunk in chunks {
        let ControlPayload::AttachmentChunk {
            attachment_id,
            chunk_index,
            data,
            ..
        } = chunk
        else {
            continue;
        };
        if attachment_id != expression_id {
            continue;
        }
        if chunk_index >= offer.chunk_count {
            continue;
        }
        let Ok(plaintext) = fek.decrypt(&data) else {
            tracing::warn!(
                community = %community_id,
                expression = %hex::encode(expression_id),
                chunk = chunk_index,
                "dropping expression chunk: FEK decrypt failed",
            );
            continue;
        };
        let Some(expected_hash) = offer.chunk_hashes.get(chunk_index as usize) else {
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
        // Re-encrypt under the same FEK so the cache's wire format stays
        // identical to what the original uploader stored.
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
        if let Err(e) = cache.insert(attachment_uuid, chunk_index, &stored, &pinned) {
            tracing::debug!(
                community = %community_id,
                chunk = chunk_index,
                error = %e,
                "cache insert during eager expression fetch failed",
            );
        }
    }
    Ok(())
}
