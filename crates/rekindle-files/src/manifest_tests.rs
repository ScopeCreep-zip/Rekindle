#![allow(
    clippy::cast_possible_truncation,
    reason = "test fixture: the constants are compile-time known and well inside range; a fallible conversion here would add noise without adding safety"
)]

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

fn sized_offer(total_size: u64, chunk_count: u32) -> AttachmentOffer {
    AttachmentOffer {
        attachment_id: [1; 16],
        filename: "x.bin".into(),
        mime_type: "application/octet-stream".into(),
        total_size,
        chunk_count,
        chunk_size: CHUNK_SIZE_BYTES as u32,
        merkle_root: [0; 32],
        chunk_hashes: vec![[1; 32]; chunk_count as usize],
        wrapped_fek: vec![0; 48],
        fek_mek_generation: 1,
    }
}

#[test]
fn validate_offer_bounds_sizes() {
    let chunk = CHUNK_SIZE_BYTES as u64;
    assert!(validate_offer(&sized_offer(0, 1)).is_ok());
    assert!(validate_offer(&sized_offer(chunk, 1)).is_ok());
    assert!(validate_offer(&sized_offer(chunk + 1, 2)).is_ok());
    assert!(matches!(
        validate_offer(&sized_offer(chunk + 1, 1)),
        Err(FilesError::InvalidManifest(_))
    ));
    assert!(matches!(
        validate_offer(&sized_offer(10, 2)),
        Err(FilesError::InvalidManifest(_))
    ));
    assert!(matches!(
        validate_offer(&sized_offer(u64::MAX, 1)),
        Err(FilesError::FileTooLarge { .. })
    ));
    let mut long_name = sized_offer(10, 1);
    long_name.filename = "a".repeat(256);
    assert!(matches!(
        validate_offer(&long_name),
        Err(FilesError::InvalidManifest(_))
    ));
}
