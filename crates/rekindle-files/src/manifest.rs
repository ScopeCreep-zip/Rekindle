//! Manifest module — re-exports the wire types defined in `rekindle-types`
//! and adds the `validate()` helper that file logic uses.
//!
//! `AttachmentOffer` and `AttachmentBitmap` themselves live in `rekindle-types`
//! (Tier 1) so the protocol layer can embed them without depending on this
//! Tier 7 crate. See plan §1.J for the design rationale.

pub use rekindle_types::attachment::{AttachmentBitmap, AttachmentOffer};

use crate::chunker::CHUNK_SIZE_BYTES;
use crate::error::FilesError;

/// Internal-consistency check: `chunk_hashes.len() == chunk_count` and
/// the recomputed Merkle root matches the announced one. Cheap; callers
/// should run this before accepting an offer from the wire. Lives here
/// (in rekindle-files) rather than on the type itself because the
/// recomputation pulls in `sha2` (only available on this crate).
pub fn validate_offer(offer: &AttachmentOffer) -> Result<(), FilesError> {
    if offer.chunk_hashes.len() != offer.chunk_count as usize {
        return Err(FilesError::OfferHashCountMismatch {
            hashes: offer.chunk_hashes.len(),
            chunk_count: offer.chunk_count,
        });
    }
    if offer.chunk_size == 0 || offer.chunk_size as usize > CHUNK_SIZE_BYTES {
        return Err(FilesError::InvalidManifest(format!(
            "chunk_size {} not in (0, {}]",
            offer.chunk_size, CHUNK_SIZE_BYTES
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rekindle_types::attachment::AttachmentOffer;

    #[test]
    fn validate_offer_catches_hash_count_mismatch() {
        let offer = AttachmentOffer {
            attachment_id: [1; 16],
            filename: "x.bin".into(),
            mime_type: "application/octet-stream".into(),
            total_size: 100,
            chunk_count: 2,
            chunk_size: CHUNK_SIZE_BYTES as u32,
            merkle_root: [0; 32],
            chunk_hashes: vec![[1; 32]], // only 1 hash but chunk_count=2
            wrapped_fek: vec![0; 48],
            fek_mek_generation: 1,
        };
        assert!(matches!(
            validate_offer(&offer),
            Err(FilesError::OfferHashCountMismatch { .. })
        ));
    }

    #[test]
    fn validate_offer_rejects_zero_chunk_size() {
        let offer = AttachmentOffer {
            attachment_id: [1; 16],
            filename: "x.bin".into(),
            mime_type: "application/octet-stream".into(),
            total_size: 0,
            chunk_count: 0,
            chunk_size: 0,
            merkle_root: [0; 32],
            chunk_hashes: vec![],
            wrapped_fek: vec![],
            fek_mek_generation: 0,
        };
        assert!(matches!(
            validate_offer(&offer),
            Err(FilesError::InvalidManifest(_))
        ));
    }
}
