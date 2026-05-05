//! Chunk + Merkle-root verification.

use sha2::{Digest, Sha256};

use rekindle_types::attachment::AttachmentOffer;

use crate::chunker::merkle_root_of;
use crate::error::FilesError;
use crate::manifest::validate_offer;

/// Verify a single decrypted (plaintext) chunk against its declared SHA-256.
///
/// Run this on every chunk received from the network *after* FEK decryption.
/// Drop the peer's contribution and re-request the chunk elsewhere on
/// failure (per plan §1.J2 transport-integrity rule).
pub fn verify_chunk(plaintext: &[u8], expected_hash: &[u8; 32]) -> Result<(), FilesError> {
    let mut hasher = Sha256::new();
    hasher.update(plaintext);
    let actual: [u8; 32] = hasher.finalize().into();
    if actual != *expected_hash {
        // index is unknown to us here — caller substitutes via .map_err if it has it
        return Err(FilesError::ChunkHashMismatch { index: u32::MAX });
    }
    Ok(())
}

/// Confirm that the offer's announced `merkle_root` matches what the
/// `chunk_hashes` list actually hashes to. Run this once per offer the
/// first time we see it (e.g. on receiving an `AttachmentOffer` over the
/// wire) — protects against a malicious announcer that lies about the
/// root while serving real chunks.
pub fn verify_merkle_root(offer: &AttachmentOffer) -> Result<(), FilesError> {
    validate_offer(offer)?;
    let recomputed = merkle_root_of(&offer.chunk_hashes);
    if recomputed != offer.merkle_root {
        return Err(FilesError::MerkleRootMismatch);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunker::{Chunker, CHUNK_SIZE_BYTES};

    fn offer_from(bytes: &[u8]) -> AttachmentOffer {
        let chunked = Chunker::chunk(bytes).unwrap();
        AttachmentOffer {
            attachment_id: chunked.attachment_id,
            filename: "f".into(),
            mime_type: "application/octet-stream".into(),
            total_size: bytes.len() as u64,
            chunk_count: chunked.chunks.len() as u32,
            chunk_size: CHUNK_SIZE_BYTES as u32,
            merkle_root: chunked.merkle_root,
            chunk_hashes: chunked.chunk_hashes,
            wrapped_fek: vec![0; 48],
            fek_mek_generation: 1,
        }
    }

    #[test]
    fn verify_merkle_root_accepts_consistent_offer() {
        let offer = offer_from(b"hello world".repeat(2000).as_slice());
        assert!(verify_merkle_root(&offer).is_ok());
    }

    #[test]
    fn verify_merkle_root_detects_tampered_root() {
        let mut offer = offer_from(b"some content".repeat(500).as_slice());
        offer.merkle_root[0] ^= 0xff;
        assert!(matches!(
            verify_merkle_root(&offer),
            Err(FilesError::MerkleRootMismatch)
        ));
    }

    #[test]
    fn verify_chunk_passes_for_correct_hash() {
        let plaintext = b"chunk plaintext bytes";
        let mut hasher = Sha256::new();
        hasher.update(plaintext);
        let hash: [u8; 32] = hasher.finalize().into();
        assert!(verify_chunk(plaintext, &hash).is_ok());
    }

    #[test]
    fn verify_chunk_fails_for_tampered_data() {
        let plaintext = b"chunk plaintext bytes";
        let mut hasher = Sha256::new();
        hasher.update(plaintext);
        let mut hash: [u8; 32] = hasher.finalize().into();
        hash[5] ^= 0x01;
        assert!(matches!(
            verify_chunk(plaintext, &hash),
            Err(FilesError::ChunkHashMismatch { .. })
        ));
    }
}
