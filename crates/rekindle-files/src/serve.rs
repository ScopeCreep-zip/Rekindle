//! Phase 15 — pure chunk-serve responder.
//!
//! Architecture §28.9 — when a peer sends a `RequestAttachment` control
//! payload, we look up each requested chunk in our local cache and
//! reply with a `MultiAttachmentChunk` envelope containing the chunks
//! we hold from the requested set. Returns the encoded reply bytes, or
//! `None` if we have nothing to offer.
//!
//! Pure function — caller supplies the locked `ChunkCache` so this
//! crate never touches `AppState`. The src-tauri facade locks
//! `state.file_caches.write()`, fetches the per-community cache, and
//! delegates here.

use rekindle_protocol::dht::community::envelope::{CommunityEnvelope, ControlPayload};
use uuid::Uuid;

use crate::cache::ChunkCache;

/// Build the reply to a `RequestAttachment` control payload.
///
/// `cache` is the per-community cache (already locked by the caller).
/// Returns `Some(encoded_envelope)` if we hold at least one of the
/// requested chunks; `None` if the request set is fully absent from
/// the cache (caller should not reply at all in that case).
#[must_use]
pub fn serve_attachment_request(
    cache: &mut ChunkCache,
    attachment_id: [u8; 16],
    requested_chunks: &[u32],
) -> Option<Vec<u8>> {
    let attachment_uuid = Uuid::from_bytes(attachment_id);

    let mut delivered: Vec<ControlPayload> = Vec::new();
    for &idx in requested_chunks {
        match cache.get(attachment_uuid, idx) {
            Ok(Some(ciphertext)) => {
                // plaintext_hash field is filled by the requester after FEK
                // decrypt; we store an all-zero placeholder over the wire.
                // (Hash verification happens against AttachmentOffer.chunk_hashes.)
                delivered.push(ControlPayload::AttachmentChunk {
                    attachment_id,
                    chunk_index: idx,
                    data: ciphertext,
                    plaintext_hash: [0u8; 32],
                });
            }
            Ok(None) => {}
            Err(e) => {
                tracing::debug!(
                    chunk = idx,
                    error = %e,
                    "cache.get failed serving attachment chunk"
                );
            }
        }
    }

    if delivered.is_empty() {
        return None;
    }
    rekindle_protocol::capnp_envelope::encode_community_envelope(&CommunityEnvelope::Control(
        ControlPayload::MultiAttachmentChunk { chunks: delivered },
    ))
    .ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::{CacheConfig, ChunkCache};
    use crate::pinned::PinnedSet;
    use tempfile::TempDir;

    fn open_cache(dir: &TempDir) -> ChunkCache {
        ChunkCache::open(CacheConfig {
            root_dir: dir.path().to_path_buf(),
            byte_budget: 1024 * 1024,
        })
        .unwrap()
    }

    fn parse_reply(bytes: &[u8]) -> Vec<(u32, Vec<u8>)> {
        let env = rekindle_protocol::capnp_envelope::decode_community_envelope(bytes)
            .expect("decode reply");
        let CommunityEnvelope::Control(ControlPayload::MultiAttachmentChunk { chunks }) = env
        else {
            panic!("unexpected reply variant");
        };
        chunks
            .into_iter()
            .filter_map(|p| match p {
                ControlPayload::AttachmentChunk {
                    chunk_index, data, ..
                } => Some((chunk_index, data)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn returns_none_when_cache_has_no_matching_chunks() {
        let temp = TempDir::new().unwrap();
        let mut cache = open_cache(&temp);
        let aid = [7u8; 16];
        let reply = serve_attachment_request(&mut cache, aid, &[0, 1, 2]);
        assert!(reply.is_none(), "empty cache should yield no reply");
    }

    #[test]
    fn returns_subset_when_cache_holds_some_chunks() {
        let temp = TempDir::new().unwrap();
        let mut cache = open_cache(&temp);
        let aid = [9u8; 16];
        let attachment_uuid = Uuid::from_bytes(aid);
        let pinned = PinnedSet::new();
        cache.insert(attachment_uuid, 0, b"chunk0", &pinned).unwrap();
        cache.insert(attachment_uuid, 2, b"chunk2", &pinned).unwrap();

        let reply = serve_attachment_request(&mut cache, aid, &[0, 1, 2, 3])
            .expect("should reply with held chunks");
        let parsed = parse_reply(&reply);
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0], (0, b"chunk0".to_vec()));
        assert_eq!(parsed[1], (2, b"chunk2".to_vec()));
    }

    #[test]
    fn returns_full_set_when_all_chunks_present() {
        let temp = TempDir::new().unwrap();
        let mut cache = open_cache(&temp);
        let aid = [42u8; 16];
        let attachment_uuid = Uuid::from_bytes(aid);
        let pinned = PinnedSet::new();
        for i in 0..3u32 {
            cache
                .insert(attachment_uuid, i, format!("chunk{i}").as_bytes(), &pinned)
                .unwrap();
        }

        let reply = serve_attachment_request(&mut cache, aid, &[0, 1, 2]).expect("full reply");
        let parsed = parse_reply(&reply);
        assert_eq!(parsed.len(), 3);
        assert_eq!(parsed[0].0, 0);
        assert_eq!(parsed[1].0, 1);
        assert_eq!(parsed[2].0, 2);
    }

    #[test]
    fn returns_none_when_requested_attachment_id_differs() {
        let temp = TempDir::new().unwrap();
        let mut cache = open_cache(&temp);
        let pinned = PinnedSet::new();
        let stored_aid = [1u8; 16];
        let stored_uuid = Uuid::from_bytes(stored_aid);
        cache.insert(stored_uuid, 0, b"data", &pinned).unwrap();

        let other_aid = [2u8; 16];
        let reply = serve_attachment_request(&mut cache, other_aid, &[0, 1]);
        assert!(reply.is_none());
    }
}
