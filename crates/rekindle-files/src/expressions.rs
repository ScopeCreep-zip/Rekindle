//! Phase 23.D.9 — Lost Cargo storage + retrieval for expression assets
//! (custom emoji, stickers, soundboard sounds). Ported from
//! `src-tauri/services/community/expression_assets.rs`. Architecture
//! §18.4 + §28.9 line 3286.
//!
//! Upload chunks bytes, encrypts each chunk under a fresh per-asset
//! FEK, stores chunks in the per-community file cache, and wraps the
//! FEK under the current community MEK. The result is an
//! `AttachmentOffer` the caller embeds in the `ExpressionAdded`
//! governance entry.
//!
//! Download materialises plaintext from the local chunk cache; returns
//! `None` (without erroring) when any chunk is missing — the
//! `eager_fetch_missing` loop refills gaps on the next merge tick.

use rekindle_crypto::group::media_key::MediaEncryptionKey;
use uuid::Uuid;

use crate::chunker::Chunker;
use crate::deps::FilesDeps;
use crate::error::FilesError;
use crate::manifest::AttachmentOffer;
use crate::CHUNK_SIZE_BYTES;

/// Chunk + encrypt + cache an expression asset, returning the
/// `AttachmentOffer` the caller embeds in `ExpressionAdded`.
///
/// `expression_id` doubles as the cache attachment_id so the chunk
/// directory layout stays stable across restarts and the eager-fetch
/// loop can probe by expression_id alone.
pub fn upload_expression_to_cache<D: FilesDeps + ?Sized>(
    deps: &D,
    community_id: &str,
    expression_id: [u8; 16],
    bytes: &[u8],
    filename: String,
    mime_type: String,
) -> Result<AttachmentOffer, FilesError> {
    let total_size = bytes.len() as u64;

    let community_mek = deps.community_mek(community_id).ok_or_else(|| {
        FilesError::MekUnavailable {
            community: community_id.to_string(),
            generation: 0,
        }
    })?;
    let mek_generation = deps.mek_generation(community_id)?;

    let fek = MediaEncryptionKey::generate(0);

    deps.ensure_cache_open(community_id)?;
    let chunked = Chunker::chunk(bytes).map_err(|e| {
        FilesError::InvalidInput(format!("chunker failed: {e}"))
    })?;
    let chunk_count = u32::try_from(chunked.chunks.len())
        .map_err(|_| FilesError::InvalidInput("chunk count exceeds u32::MAX".to_string()))?;
    let attachment_uuid = Uuid::from_bytes(expression_id);

    let chunks = chunked.chunks.clone();
    let fek_clone = fek.clone();
    deps.with_cache_mut(community_id, &mut move |cache, pinned| {
        for (idx, chunk) in chunks.iter().enumerate() {
            let ciphertext = fek_clone
                .encrypt(chunk)
                .map_err(|e| FilesError::Encrypt(format!("FEK encrypt chunk {idx}: {e}")))?;
            let chunk_idx = u32::try_from(idx).map_err(|_| {
                FilesError::InvalidInput("chunk index exceeds u32::MAX".to_string())
            })?;
            cache
                .insert(attachment_uuid, chunk_idx, &ciphertext, pinned)
                .map_err(|e| {
                    FilesError::InvalidInput(format!("cache insert chunk {chunk_idx}: {e}"))
                })?;
        }
        Ok(())
    })?;

    let wrapped_fek = community_mek
        .encrypt(fek.as_bytes())
        .map_err(|e| FilesError::Encrypt(format!("wrap FEK: {e}")))?;

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

/// Materialise the plaintext bytes for an expression asset from the
/// local chunk cache. Returns `None` (without erroring) when any chunk
/// is missing — the eager-fetch loop will fill the gap on the next
/// merge.
///
/// For now, only the *current* community MEK can unwrap. A full
/// implementation would walk historical MEKs by `fek_mek_generation`;
/// expressions uploaded under an old MEK will read as missing until
/// re-uploaded after a rotation. Architecture §7 leaves the
/// historical-MEK store as a follow-on for cross-rotation playback.
pub fn read_expression_bytes<D: FilesDeps + ?Sized>(
    deps: &D,
    community_id: &str,
    offer: &AttachmentOffer,
) -> Option<Vec<u8>> {
    let community_mek = deps.community_mek(community_id)?;
    let raw_fek = community_mek.decrypt(&offer.wrapped_fek).ok()?;
    let fek_bytes: [u8; 32] = raw_fek.as_slice().try_into().ok()?;
    let fek = MediaEncryptionKey::from_bytes(fek_bytes, 0);

    let attachment_uuid = Uuid::from_bytes(offer.attachment_id);
    let total_size = usize::try_from(offer.total_size).unwrap_or(0);
    let chunk_count = offer.chunk_count;

    let mut plaintext: Option<Vec<u8>> = None;
    let _ = deps.with_cache_mut(community_id, &mut |cache, _pinned| {
        let mut out = Vec::with_capacity(total_size);
        for idx in 0..chunk_count {
            let Some(ciphertext) = cache.get(attachment_uuid, idx).ok().flatten() else {
                return Ok(());
            };
            let Ok(chunk) = fek.decrypt(&ciphertext) else {
                return Ok(());
            };
            out.extend_from_slice(&chunk);
        }
        plaintext = Some(out);
        Ok(())
    });
    plaintext
}
