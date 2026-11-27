//! Manifest module — re-exports the wire types defined in `rekindle-types`
//! and adds the `validate()` helper that file logic uses.
//!
//! `AttachmentOffer` and `AttachmentBitmap` themselves live in `rekindle-types`
//! (Tier 1) so the protocol layer can embed them without depending on this
//! Tier 7 crate. See plan §1.J for the design rationale.

pub use rekindle_types::attachment::{AttachmentBitmap, AttachmentOffer};

use crate::chunker::{CHUNK_SIZE_BYTES, MAX_FILE_SIZE_BYTES};
use crate::error::FilesError;

/// Longest attachment filename accepted from an offer, in bytes.
pub const MAX_OFFER_FILENAME_BYTES: usize = 255;

/// Internal-consistency and bounds check on an offer from the wire. Cheap;
/// callers run it (with `verify::verify_merkle_root`) before acting on an
/// offer. Every size the download path allocates from is bounded here:
/// - `chunk_hashes.len() == chunk_count`;
/// - `chunk_size` in `(0, CHUNK_SIZE_BYTES]`;
/// - `total_size <= MAX_FILE_SIZE_BYTES`;
/// - `chunk_count` is exactly what `total_size` needs (one chunk for an
///   empty file, as `Chunker::chunk` produces);
/// - the filename is at most [`MAX_OFFER_FILENAME_BYTES`].
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
    if offer.total_size > MAX_FILE_SIZE_BYTES {
        return Err(FilesError::FileTooLarge {
            actual: offer.total_size,
            max: MAX_FILE_SIZE_BYTES,
        });
    }
    let expected_chunks = offer
        .total_size
        .div_ceil(u64::from(offer.chunk_size))
        .max(1);
    if u64::from(offer.chunk_count) != expected_chunks {
        return Err(FilesError::InvalidManifest(format!(
            "chunk_count {} does not fit total_size {} at chunk_size {}",
            offer.chunk_count, offer.total_size, offer.chunk_size
        )));
    }
    if offer.filename.len() > MAX_OFFER_FILENAME_BYTES {
        return Err(FilesError::InvalidManifest(format!(
            "filename is {} bytes, max {MAX_OFFER_FILENAME_BYTES}",
            offer.filename.len()
        )));
    }
    Ok(())
}

#[cfg(test)]
#[path = "manifest_tests.rs"]
mod tests;
