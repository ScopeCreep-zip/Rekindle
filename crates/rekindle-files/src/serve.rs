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

use rekindle_codec::community::envelope::{CommunityEnvelope, ControlPayload};
use rekindle_types::dht_layout::MAX_APP_MESSAGE_LEN;
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
    let mut reply: Option<Vec<u8>> = None;
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
                // The reply is one app-call answer, bounded by Veilid's
                // message limit: it carries the chunks that fit, and the
                // requester asks again for the rest.
                match encode(&delivered) {
                    Some(bytes) if bytes.len() <= MAX_APP_MESSAGE_LEN => reply = Some(bytes),
                    _ => break,
                }
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

    reply
}

/// The `MultiAttachmentChunk` envelope carrying `chunks`.
fn encode(chunks: &[ControlPayload]) -> Option<Vec<u8>> {
    rekindle_codec::capnp_envelope::encode_community_envelope(&CommunityEnvelope::control(
        ControlPayload::MultiAttachmentChunk {
            chunks: chunks.to_vec(),
        },
    ))
    .ok()
}

#[cfg(test)]
#[path = "serve_tests.rs"]
mod tests;
