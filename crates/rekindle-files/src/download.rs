//! Phase 15 — Lost Cargo download orchestration.
//!
//! Architecture §28.9 — fetch the AttachmentOffer from the channel
//! SMPL record, unwrap the FEK using the historical channel MEK that
//! wrapped it, discover peers advertising chunks via AttachmentCached
//! entries, request missing chunks from each source in arrival order,
//! verify each chunk against the offer's plaintext SHA-256, cache the
//! re-encrypted ciphertext, reassemble the plaintext + write to disk,
//! advertise full possession, and persist the local path so the UI
//! flips the message bubble from "Download" to "Open".
//!
//! Parameterised over `FilesDeps`. The src-tauri `FilesAdapter`
//! supplies the concrete DHT + transport + DB + cache + governance
//! wiring.

use std::path::Path;

use rekindle_protocol::dht::community::envelope::{CommunityEnvelope, ControlPayload};
use rekindle_protocol::dht::community::permissions_v2::Permissions;
use uuid::Uuid;

use crate::deps::FilesDeps;
use crate::dht_scan::{discover_sources_in_entries, fetch_offer_in_entries, DiscoveredSource};
use crate::error::FilesError;
use crate::fek::unwrap_fek_for_offer;
use crate::manifest::validate_offer;
use crate::verify::{verify_chunk, verify_merkle_root};
use rekindle_protocol::dht::community::channel_record::ChannelAttachmentCached;
use rekindle_types::attachment::{AttachmentBitmap, AttachmentOffer};

/// Download an attachment by hex id to `save_path`. v1 strategy: ask
/// each discovered source in turn for missing chunks; verify each
/// chunk against the offer's hash list; reassemble + write to disk;
/// advertise full possession via `AttachmentCached`.
pub async fn download_attachment<D: FilesDeps>(
    deps: &D,
    community_id: &str,
    channel_id: &str,
    attachment_id_hex: &str,
    save_path: &Path,
) -> Result<(), FilesError> {
    deps.require_permission(community_id, Permissions::READ_MESSAGE_HISTORY)?;
    deps.ensure_cache_open(community_id)?;

    let offer = fetch_offer(deps, community_id, channel_id, attachment_id_hex).await?;
    verify_merkle_root(&offer)
        .map_err(|e| FilesError::OfferInvalid(format!("merkle root: {e}")))?;
    validate_offer(&offer).map_err(|e| FilesError::OfferInvalid(e.to_string()))?;
    let attachment_id = offer.attachment_id;
    let attachment_uuid = Uuid::from_bytes(attachment_id);
    let chunk_count = offer.chunk_count;

    // Load + unwrap FEK using the historical channel MEK that wrapped it.
    let channel_mek = deps
        .historical_channel_mek(community_id, channel_id, offer.fek_mek_generation)
        .ok_or(FilesError::MekUnavailable {
            community: community_id.to_string(),
            generation: offer.fek_mek_generation,
        })?;
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

    let sources = discover_sources(deps, community_id, channel_id, attachment_id, chunk_count)
        .await?;
    if sources.is_empty() {
        return Err(FilesError::NotFound(
            "no peers advertise this attachment — try again when at least one source is online"
                .into(),
        ));
    }

    // Walk sources in arrival order; for each, request the chunks they
    // have that we still need.
    for src in &sources {
        let needed: Vec<u32> = src.bitmap.intersect(&inverse(&have));
        if needed.is_empty() {
            continue;
        }
        let response =
            send_chunk_request(deps, community_id, channel_id, attachment_id, &needed, src)
                .await?;
        for chunk in response {
            // Decrypt then verify against the offer's plaintext SHA-256.
            let plaintext = fek.decrypt(&chunk.data).map_err(|e| {
                FilesError::Decrypt(format!("chunk {} decrypt: {e}", chunk.chunk_index))
            })?;
            let expected = offer.chunk_hashes.get(chunk.chunk_index as usize).ok_or(
                FilesError::ChunkIndexOutOfRange {
                    index: chunk.chunk_index,
                    chunk_count,
                },
            )?;
            if let Err(e) = verify_chunk(&plaintext, expected) {
                tracing::warn!(
                    community = %community_id,
                    peer = %src.pseudonym,
                    chunk = chunk.chunk_index,
                    error = %e,
                    "dropping malformed chunk; will re-request"
                );
                continue;
            }
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
            have.set(chunk.chunk_index);
        }
        if have.is_complete() {
            break;
        }
    }

    if !have.is_complete() {
        return Err(FilesError::DownloadIncomplete {
            have: have.count(),
            total: chunk_count,
        });
    }

    // Reassemble plaintext + write to disk. Uses `with_cache_mut`
    // because ChunkCache::get takes `&mut self` (LRU touch).
    let mut out: Vec<u8> = Vec::with_capacity(usize::try_from(offer.total_size).unwrap_or(0));
    {
        let mut buffer: Option<Vec<u8>> = Some(Vec::with_capacity(out.capacity()));
        deps.with_cache_mut(community_id, &mut |cache, _pinned| {
            let buf = buffer.as_mut().expect("buffer present");
            for idx in 0..chunk_count {
                let ciphertext = cache
                    .get(attachment_uuid, idx)
                    .map_err(|e| FilesError::Db(format!("cache get {idx}: {e}")))?
                    .ok_or_else(|| {
                        FilesError::NotFound(format!("chunk {idx} missing mid-reassembly"))
                    })?;
                let plaintext = fek.decrypt(&ciphertext).map_err(|e| {
                    FilesError::Decrypt(format!("reassembly {idx}: {e}"))
                })?;
                buf.extend_from_slice(&plaintext);
            }
            Ok(())
        })?;
        out = buffer.unwrap_or_default();
    }
    if let Some(parent) = save_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| FilesError::Io {
            path: parent.display().to_string(),
            source: e,
        })?;
    }
    std::fs::write(save_path, &out).map_err(|e| FilesError::Io {
        path: save_path.display().to_string(),
        source: e,
    })?;

    // Advertise full possession to the swarm.
    let channel_log_key = deps.channel_log_key(community_id, channel_id)?;
    let slot_keypair = deps.slot_keypair(community_id)?;
    let slot_index = deps.my_subkey_index(community_id)?;
    let sender_pseudonym = deps.my_pseudonym(community_id)?;
    let cached = ChannelAttachmentCached {
        attachment_id,
        chunk_bitmap: AttachmentBitmap::full(chunk_count).as_bytes().to_vec(),
        chunk_count,
        author_pseudonym: sender_pseudonym,
        lamport_ts: deps.increment_lamport(community_id),
    };
    deps.write_attachment_cached_to_smpl(
        community_id,
        &channel_log_key,
        slot_index,
        &slot_keypair,
        &cached,
    )
    .await?;

    // Update SQLite row's local_path so the UI flips to "Open" instead of "Download".
    let owner_key = deps.owner_key()?;
    deps.persist_local_path(&owner_key, channel_id, attachment_id_hex, save_path)
        .await?;

    Ok(())
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

/// Fetch the offer for an attachment from the channel SMPL record by
/// scanning all 255 member subkeys for a Message entry carrying it.
async fn fetch_offer<D: FilesDeps>(
    deps: &D,
    community_id: &str,
    channel_id: &str,
    attachment_id_hex: &str,
) -> Result<AttachmentOffer, FilesError> {
    let target_attachment_id = parse_attachment_id_hex(attachment_id_hex)?;
    let channel_log_key = deps.channel_log_key(community_id, channel_id)?;
    let entries = deps.scan_channel_subkeys(&channel_log_key).await?;
    fetch_offer_in_entries(&entries, target_attachment_id).ok_or_else(|| {
        FilesError::NotFound(format!(
            "attachment {attachment_id_hex} not found in channel {channel_id}"
        ))
    })
}

/// Discover peers advertising chunks of an attachment via
/// `AttachmentCached` entries across all 255 member subkeys.
async fn discover_sources<D: FilesDeps>(
    deps: &D,
    community_id: &str,
    channel_id: &str,
    attachment_id: [u8; 16],
    chunk_count: u32,
) -> Result<Vec<DiscoveredSource>, FilesError> {
    let channel_log_key = deps.channel_log_key(community_id, channel_id)?;
    let entries = deps.scan_channel_subkeys(&channel_log_key).await?;
    Ok(discover_sources_in_entries(&entries, attachment_id, chunk_count))
}

/// Ask a single peer for a subset of chunks. Encodes the
/// `RequestAttachment` envelope, app_calls the peer's route, decodes
/// the reply, and unwraps the chunks.
async fn send_chunk_request<D: FilesDeps>(
    deps: &D,
    community_id: &str,
    channel_id: &str,
    attachment_id: [u8; 16],
    requested_chunks: &[u32],
    source: &DiscoveredSource,
) -> Result<Vec<ChunkResponse>, FilesError> {
    let route_blob = deps
        .peer_route_blob(community_id, &source.pseudonym)
        .ok_or_else(|| {
            FilesError::NotFound(format!(
                "source peer {} not online — cannot app_call",
                source.pseudonym
            ))
        })?;

    let requester_pseudonym = deps.my_pseudonym(community_id)?;
    let payload = CommunityEnvelope::Control(ControlPayload::RequestAttachment {
        channel_id: channel_id.to_string(),
        attachment_id,
        requested_chunks: requested_chunks.to_vec(),
        requester_pseudonym,
    });
    let bytes = rekindle_protocol::capnp_envelope::encode_community_envelope(&payload)
        .map_err(|e| FilesError::Transport(format!("encode RequestAttachment: {e}")))?;

    let reply = deps.app_call_peer(&route_blob, bytes).await?;
    let envelope = rekindle_protocol::capnp_envelope::decode_community_envelope(&reply)
        .map_err(|e| FilesError::Transport(format!("decode reply: {e}")))?;
    match envelope {
        CommunityEnvelope::Control(ControlPayload::MultiAttachmentChunk { chunks }) => Ok(chunks
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
mod tests {
    use super::*;
    use crate::test_mock::MockDeps;
    use rekindle_protocol::dht::community::channel_record::{ChannelMessage, ChannelRecordEntry};

    fn build_offer_entry(attachment_id: [u8; 16], chunk_count: u32) -> ChannelRecordEntry {
        ChannelRecordEntry::Message(ChannelMessage {
            sequence: 1,
            sender_pseudonym: "owner".into(),
            ciphertext: Vec::new(),
            mek_generation: 0,
            timestamp: 0,
            reply_to: None,
            lamport_ts: 1,
            message_id: Some("m1".into()),
            attachment: Some(AttachmentOffer {
                attachment_id,
                filename: "f.bin".into(),
                mime_type: "application/octet-stream".into(),
                total_size: 0,
                chunk_count,
                chunk_size: 0,
                merkle_root: [0u8; 32],
                chunk_hashes: vec![[0u8; 32]; chunk_count as usize],
                wrapped_fek: Vec::new(),
                fek_mek_generation: 0,
            }),
            flags: 0,
            mentioned_pseudonyms: Vec::new(),
            mentioned_roles: Vec::new(),
        })
    }

    #[tokio::test]
    async fn invalid_attachment_id_hex_is_rejected() {
        let deps = MockDeps::new("c1", "ch1");
        let err = download_attachment(&deps, "c1", "ch1", "not-hex", std::path::Path::new("/tmp/x"))
            .await
            .unwrap_err();
        assert!(matches!(err, FilesError::InvalidAttachmentId(_)));
    }

    #[tokio::test]
    async fn offer_not_found_returns_not_found() {
        let deps = MockDeps::new("c1", "ch1"); // no entries
        let err = download_attachment(
            &deps,
            "c1",
            "ch1",
            &"de".repeat(16),
            std::path::Path::new("/tmp/x"),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, FilesError::NotFound(msg) if msg.contains("not found")));
    }

    #[tokio::test]
    async fn permission_denied_blocks_download() {
        let mut deps = MockDeps::new("c1", "ch1");
        deps.permission_pass = false;
        let err = download_attachment(
            &deps,
            "c1",
            "ch1",
            &"00".repeat(16),
            std::path::Path::new("/tmp/x"),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, FilesError::PermissionDenied(_)));
    }

    #[tokio::test]
    async fn offer_with_corrupt_merkle_root_rejected() {
        // Pre-populate entries with a Message carrying an offer whose
        // chunk_hashes are all-zero but merkle_root claims something
        // else → verify_merkle_root fails.
        let aid = [3u8; 16];
        let deps = MockDeps::new("c1", "ch1").with_entries(vec![build_offer_entry(aid, 2)]);
        // Tamper: make the offer's merkle_root non-zero so it doesn't
        // match the empty-hash recomputation.
        if let Some(ChannelRecordEntry::Message(msg)) = deps.channel_entries.lock().get_mut(0) {
            if let Some(o) = msg.attachment.as_mut() {
                o.merkle_root = [99u8; 32];
            }
        }
        let err = download_attachment(
            &deps,
            "c1",
            "ch1",
            &hex::encode(aid),
            std::path::Path::new("/tmp/x"),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, FilesError::OfferInvalid(_)));
    }
}
