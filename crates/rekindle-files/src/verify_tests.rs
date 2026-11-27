#![allow(
    clippy::cast_possible_truncation,
    reason = "test fixture: the constants are compile-time known and well inside range; a fallible conversion here would add noise without adding safety"
)]

use super::*;
use crate::chunker::{Chunker, CHUNK_SIZE_BYTES};

fn offer_from(bytes: &[u8]) -> AttachmentOffer {
    let chunked = Chunker::chunk(bytes, [1; 16]).unwrap();
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
