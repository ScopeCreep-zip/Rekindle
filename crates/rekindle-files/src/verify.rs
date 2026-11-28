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
#[path = "verify_tests.rs"]
mod tests;
