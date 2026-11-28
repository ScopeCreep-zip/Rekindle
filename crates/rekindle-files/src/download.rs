//! Phase 15 — Lost Cargo download orchestration.
//!
//! Architecture §28.9, in three steps:
//! - [`fetch_attachment_to_cache`]: fetch the AttachmentOffer from the
//!   channel SMPL record, unwrap the FEK using the historical channel MEK
//!   that wrapped it, discover peers advertising chunks via
//!   AttachmentCached entries, request missing chunks from each source in
//!   arrival order, verify each chunk against the offer's plaintext
//!   SHA-256, cache the re-encrypted ciphertext, and advertise full
//!   possession;
//! - [`assemble_attachment`]: decrypt the cached chunks into the file;
//! - [`download_attachment`]: both, then write the file to disk and record
//!   its path so "Show in folder" can find it.
//!
//! Voice messages play from [`assemble_attachment`]'s bytes and never touch
//! the filesystem outside the chunk cache.
//!
//! Parameterised over `FilesDeps`. The Host's `HostFiles`
//! supplies the concrete DHT + transport + DB + cache + governance
//! wiring.

use std::path::Path;

use rekindle_codec::community::envelope::{CommunityEnvelope, ControlPayload};
use rekindle_crypto::group::media_key::MediaEncryptionKey;
use rekindle_types::permissions;
use uuid::Uuid;

use crate::deps::{ChannelPost, FilesDeps, FilesEvent};
use crate::dht_scan::{discover_sources_in_entries, fetch_offer_in_entries, DiscoveredSource};
use crate::error::FilesError;
use crate::fek::unwrap_fek_for_offer;
use crate::manifest::validate_offer;
use crate::verify::{verify_chunk, verify_merkle_root};
use rekindle_types::attachment::{AttachmentBitmap, AttachmentOffer};
use rekindle_types::message::PostEntry;

/// An attachment whose chunks are all in the local cache, with the key
/// that decrypts them.
pub struct FetchedAttachment {
    pub offer: AttachmentOffer,
    fek: MediaEncryptionKey,
}

/// Bring every chunk of an attachment into the local cache.
///
/// Reads and validates the offer, unwraps its FEK with the historical
/// channel MEK, and, unless the cache already holds every chunk, asks each
/// advertising peer in arrival order for the chunks still missing. Each
/// chunk is verified against the offer's plaintext SHA-256 before it is
/// cached. On completing a fetch, advertises full possession with an
/// `AttachmentCached` entry so other members can fetch from us.
pub async fn fetch_attachment_to_cache<D: FilesDeps>(
    deps: &D,
    community_id: &str,
    channel_id: &str,
    attachment_id_hex: &str,
) -> Result<FetchedAttachment, FilesError> {
    deps.require_permission(community_id, Some(channel_id), permissions::READ_HISTORY)?;
    deps.ensure_cache_open(community_id)?;

    let offer = fetch_offer(deps, community_id, channel_id, attachment_id_hex).await?;
    verify_merkle_root(&offer)
        .map_err(|e| FilesError::OfferInvalid(format!("merkle root: {e}")))?;
    validate_offer(&offer).map_err(|e| FilesError::OfferInvalid(e.to_string()))?;
    let attachment_id = offer.attachment_id;
    let attachment_uuid = Uuid::from_bytes(attachment_id);
    let chunk_count = offer.chunk_count;

    // Load + unwrap FEK using the historical channel MEK that wrapped it.
    let channel_mek =
        crate::keys::text_key_at(deps, community_id, channel_id, offer.fek_mek_generation)?;
    let fek = unwrap_fek_for_offer(&channel_mek, &offer)?;

    // Compute current-bitmap of what we already have locally.
    let mut have: AttachmentBitmap = {
        let mut bitmap: Option<AttachmentBitmap> = None;
        deps.with_cache(community_id, &mut |cache| {
            bitmap = Some(
                cache
                    .bitmap_for(attachment_uuid, chunk_count)
                    .map_err(|e| FilesError::Db(format!("bitmap_for: {e}")))?,
            );
            Ok(())
        })?;
        bitmap.ok_or_else(|| FilesError::NotFound(format!("cache for {community_id}")))?
    };
    // The checkpoint is what the cache holds, even when it holds every
    // chunk already (a transfer resumed after its last chunk was cached).
    chunk_cached(deps, community_id, &offer, &have);
    if have.is_complete() {
        return Ok(FetchedAttachment { offer, fek });
    }

    let sources =
        discover_sources(deps, community_id, channel_id, attachment_id, chunk_count).await?;
    if sources.is_empty() {
        return Err(FilesError::NotFound(
            "no peers advertise this attachment — try again when at least one source is online"
                .into(),
        ));
    }

    // Walk sources in arrival order (§28.9: every advertising peer is
    // asked); for each, request the chunks they have that we still need.
    // A source that is offline or does not answer is recorded and the walk
    // moves on to the next one, so the attempt fails only when no source
    // completes it.
    let mut failed_sources = 0usize;
    for src in &sources {
        let answered =
            fetch_from_source(deps, community_id, channel_id, &offer, &fek, src, &mut have).await?;
        if !answered {
            failed_sources += 1;
        }
        if have.is_complete() {
            break;
        }
    }

    if !have.is_complete() {
        tracing::debug!(
            failed_sources,
            sources = sources.len(),
            have = have.count(),
            total = chunk_count,
            "attachment download attempt incomplete after asking every source"
        );
        return Err(FilesError::DownloadIncomplete {
            have: have.count(),
            total: chunk_count,
        });
    }

    // Advertise full possession to the swarm: an `AttachmentCached`
    // entry on our channel outbox (plan E3.3 slice 7).
    let possession = ChannelPost {
        governance_cut: Vec::new(),
        community: community_id.to_owned(),
        channel: channel_id.to_owned(),
        message: format!("cached-{}", Uuid::new_v4()),
        entry: PostEntry::Cached {
            attachment_id,
            chunk_count,
        },
    };
    deps.queue_post(&possession).await?;

    Ok(FetchedAttachment { offer, fek })
}

/// Decrypt and concatenate a fetched attachment's cached chunks. The
/// result is exactly `offer.total_size` bytes; `validate_offer` bounds it
/// by `MAX_FILE_SIZE_BYTES` before anything is allocated.
pub fn assemble_attachment<D: FilesDeps>(
    deps: &D,
    community_id: &str,
    fetched: &FetchedAttachment,
) -> Result<Vec<u8>, FilesError> {
    let offer = &fetched.offer;
    let attachment_uuid = Uuid::from_bytes(offer.attachment_id);
    let total = usize::try_from(offer.total_size)
        .map_err(|_| FilesError::OfferInvalid("total_size exceeds memory".into()))?;
    let mut out: Vec<u8> = Vec::with_capacity(total);
    // `with_cache_mut` because ChunkCache::get takes `&mut self` (LRU touch).
    deps.with_cache_mut(community_id, &mut |cache, _pinned| {
        for idx in 0..offer.chunk_count {
            let ciphertext = cache
                .get(attachment_uuid, idx)
                .map_err(|e| FilesError::Db(format!("cache get {idx}: {e}")))?
                .ok_or_else(|| {
                    FilesError::NotFound(format!("chunk {idx} missing mid-reassembly"))
                })?;
            let plaintext = fetched
                .fek
                .decrypt(&ciphertext)
                .map_err(|e| FilesError::Decrypt(format!("reassembly {idx}: {e}")))?;
            out.extend_from_slice(&plaintext);
        }
        Ok(())
    })?;
    if out.len() != total {
        return Err(FilesError::OfferInvalid(format!(
            "assembled {} bytes, offer says {total}",
            out.len()
        )));
    }
    Ok(out)
}

/// Fetch an attachment, write it to `save_path`, record the path on the
/// message row, and emit [`FilesEvent::AttachmentDownloaded`].
pub async fn download_attachment<D: FilesDeps>(
    deps: &D,
    community_id: &str,
    channel_id: &str,
    attachment_id_hex: &str,
    save_path: &Path,
) -> Result<(), FilesError> {
    let fetched =
        fetch_attachment_to_cache(deps, community_id, channel_id, attachment_id_hex).await?;
    let bytes = assemble_attachment(deps, community_id, &fetched)?;
    if let Some(parent) = save_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| FilesError::Io {
                path: parent.display().to_string(),
                source: e,
            })?;
    }
    tokio::fs::write(save_path, &bytes)
        .await
        .map_err(|e| FilesError::Io {
            path: save_path.display().to_string(),
            source: e,
        })?;

    // The message row keeps the path so "Show in folder" can find the file.
    let owner_key = deps.owner_key()?;
    deps.persist_local_path(&owner_key, channel_id, attachment_id_hex, save_path)
        .await?;
    deps.emit_event(FilesEvent::AttachmentDownloaded {
        community_id: community_id.to_string(),
        channel_id: channel_id.to_string(),
        attachment_id_hex: attachment_id_hex.to_string(),
    });
    Ok(())
}

/// Ask one source for the chunks it advertises that we still lack, until
/// it delivers nothing new. One reply carries only the chunks that fit
/// Veilid's app-call limit (`serve_attachment_request`), so a source is
/// asked again while it still delivers chunks we lack.
///
/// `Ok(false)` records a source that failed (offline, the call failed or
/// timed out, or its reply did not decode); the caller asks the next one.
/// `Err` is a local failure (our pseudonym, encoding, the cache) that no
/// other source can fix. Each chunk is verified against the offer's
/// plaintext SHA-256 before it is cached; a chunk that does not decrypt,
/// is out of range or fails its hash is dropped and stays missing.
async fn fetch_from_source<D: FilesDeps>(
    deps: &D,
    community_id: &str,
    channel_id: &str,
    offer: &AttachmentOffer,
    fek: &MediaEncryptionKey,
    src: &DiscoveredSource,
    have: &mut AttachmentBitmap,
) -> Result<bool, FilesError> {
    let attachment_uuid = Uuid::from_bytes(offer.attachment_id);
    loop {
        let needed: Vec<u32> = src.bitmap.intersect(&inverse(have));
        if needed.is_empty() {
            return Ok(true);
        }
        let held_before = have.count();
        let request =
            encode_chunk_request(deps, community_id, channel_id, offer.attachment_id, &needed)?;
        let response = match ask_source(deps, community_id, src, request).await {
            Ok(response) => response,
            Err(e) => {
                tracing::debug!(
                    reason = source_failure_kind(&e),
                    "attachment source failed; asking the next source"
                );
                return Ok(false);
            }
        };
        for chunk in response {
            let Some(plaintext) = verified_plaintext(offer, fek, &chunk) else {
                continue;
            };
            // Re-encrypt with FEK for cache storage (so we can serve the
            // same wire format back to other peers without re-keying).
            let stored = fek
                .encrypt(&plaintext)
                .map_err(|e| FilesError::Encrypt(format!("re-encrypt cache: {e}")))?;
            let chunk_index = chunk.chunk_index;
            deps.with_cache_mut(community_id, &mut |cache, pinned| {
                cache
                    .insert(attachment_uuid, chunk_index, &stored, pinned)
                    .map_err(|e| FilesError::Db(format!("cache insert {chunk_index}: {e}")))
            })?;
            have.set(chunk_index);
            chunk_cached(deps, community_id, offer, have);
        }
        if have.count() == held_before {
            return Ok(true);
        }
    }
}

/// Tell the deps the fetch's checkpoint: `have`'s chunks, and their bytes
/// (every chunk is `chunk_size` but the last, so the held bytes are capped
/// by `total_size`).
fn chunk_cached<D: FilesDeps>(
    deps: &D,
    community_id: &str,
    offer: &AttachmentOffer,
    have: &AttachmentBitmap,
) {
    let chunks_done = have.count();
    deps.emit_event(FilesEvent::ChunkCached {
        community_id: community_id.to_owned(),
        attachment_id_hex: hex::encode(offer.attachment_id),
        chunks_done,
        chunk_count: offer.chunk_count,
        bytes_done: (u64::from(chunks_done) * u64::from(offer.chunk_size)).min(offer.total_size),
        total: offer.total_size,
    });
}

/// A served chunk's plaintext, if it decrypts under the FEK, is in range
/// and matches the offer's plaintext SHA-256; `None` drops it.
fn verified_plaintext(
    offer: &AttachmentOffer,
    fek: &MediaEncryptionKey,
    chunk: &ChunkResponse,
) -> Option<Vec<u8>> {
    let Some(expected) = offer.chunk_hashes.get(chunk.chunk_index as usize) else {
        tracing::warn!("dropping out-of-range attachment chunk");
        return None;
    };
    let plaintext = match fek.decrypt(&chunk.data) {
        Ok(plaintext) => plaintext,
        Err(e) => {
            tracing::warn!(error = %e, "dropping attachment chunk that does not decrypt");
            return None;
        }
    };
    if let Err(e) = verify_chunk(&plaintext, expected) {
        tracing::warn!(error = %e, "dropping malformed attachment chunk; will re-request");
        return None;
    }
    Some(plaintext)
}

/// The failure class of a source, for logs that carry no identifiers.
fn source_failure_kind(e: &FilesError) -> &'static str {
    match e {
        FilesError::NotFound(_) => "offline",
        FilesError::Transport(_) => "transport",
        _ => "other",
    }
}

#[derive(Debug, Clone)]
struct ChunkResponse {
    chunk_index: u32,
    data: Vec<u8>,
}

/// Inverse of a bitmap — bits set where the input has gaps.
fn inverse(bm: &AttachmentBitmap) -> AttachmentBitmap {
    let count = bm.chunk_count();
    let mut out = AttachmentBitmap::new(count);
    for i in 0..count {
        if !bm.has(i) {
            out.set(i);
        }
    }
    out
}

/// Fetch the offer for an attachment from the channel's entries: a
/// Message entry carrying it.
async fn fetch_offer<D: FilesDeps>(
    deps: &D,
    community_id: &str,
    channel_id: &str,
    attachment_id_hex: &str,
) -> Result<AttachmentOffer, FilesError> {
    let target_attachment_id = parse_attachment_id_hex(attachment_id_hex)?;
    let entries = deps.channel_entries(community_id, channel_id).await?;
    fetch_offer_in_entries(&entries, target_attachment_id).ok_or_else(|| {
        FilesError::NotFound(format!(
            "attachment {attachment_id_hex} not found in channel {channel_id}"
        ))
    })
}

/// Discover peers advertising chunks of an attachment via
/// the channel's `AttachmentCached` entries.
async fn discover_sources<D: FilesDeps>(
    deps: &D,
    community_id: &str,
    channel_id: &str,
    attachment_id: [u8; 16],
    chunk_count: u32,
) -> Result<Vec<DiscoveredSource>, FilesError> {
    let entries = deps.channel_entries(community_id, channel_id).await?;
    Ok(discover_sources_in_entries(
        &entries,
        attachment_id,
        chunk_count,
    ))
}

/// Encode the `RequestAttachment` envelope for `requested_chunks`.
fn encode_chunk_request<D: FilesDeps>(
    deps: &D,
    community_id: &str,
    channel_id: &str,
    attachment_id: [u8; 16],
    requested_chunks: &[u32],
) -> Result<Vec<u8>, FilesError> {
    let requester_pseudonym = deps.my_pseudonym(community_id)?;
    let payload = CommunityEnvelope::control(ControlPayload::RequestAttachment {
        channel_id: channel_id.to_string(),
        attachment_id,
        requested_chunks: requested_chunks.to_vec(),
        requester_pseudonym,
    });
    rekindle_codec::capnp_envelope::encode_community_envelope(&payload)
        .map_err(|e| FilesError::Transport(format!("encode RequestAttachment: {e}")))
}

/// Ask a single peer for a subset of chunks: app_call the peer's route
/// with the encoded request, decode the reply, and unwrap the chunks.
/// Every error is the source's (offline, call failed, reply undecodable).
async fn ask_source<D: FilesDeps>(
    deps: &D,
    community_id: &str,
    source: &DiscoveredSource,
    request: Vec<u8>,
) -> Result<Vec<ChunkResponse>, FilesError> {
    let route_blob = deps
        .peer_route_blob(community_id, &source.pseudonym)
        .ok_or_else(|| FilesError::NotFound("source peer not online".into()))?;
    let reply = deps.app_call_peer(&route_blob, request).await?;
    let envelope = rekindle_codec::capnp_envelope::decode_community_envelope(&reply)
        .map_err(|e| FilesError::Transport(format!("decode reply: {e}")))?;
    match envelope.into_control() {
        Some(ControlPayload::MultiAttachmentChunk { chunks }) => Ok(chunks
            .into_iter()
            .filter_map(|c| match c {
                ControlPayload::AttachmentChunk {
                    chunk_index, data, ..
                } => Some(ChunkResponse { chunk_index, data }),
                _ => None,
            })
            .collect()),
        _ => Ok(Vec::new()),
    }
}

fn parse_attachment_id_hex(hex_str: &str) -> Result<[u8; 16], FilesError> {
    hex::decode(hex_str)
        .ok()
        .and_then(|b| b.try_into().ok())
        .ok_or_else(|| FilesError::InvalidAttachmentId(hex_str.to_string()))
}

#[cfg(test)]
mod sources_tests;

#[cfg(test)]
#[path = "download_tests.rs"]
mod tests;
