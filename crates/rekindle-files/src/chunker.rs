//! Chunk a file's plaintext bytes for Lost Cargo distribution.
//!
//! Spec §28.9 line 3231 fixes the chunk size at "≤28 KB" — the Veilid
//! `app_message` payload limit minus protocol overhead. We use exactly
//! 28 KB so the math is uniform.

use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::error::FilesError;

/// Spec §28.9 line 3231.
pub const CHUNK_SIZE_BYTES: usize = 28 * 1024;

/// v1 cap (see plan §1.J6): the flat `chunk_hashes` list must fit in a
/// 32 KB SMPL subkey. 1024 chunks × 32 bytes = 32 KB exact; we keep a
/// safety margin and cap at 1000 chunks → 28 MB. Larger files become
/// BEP-52 binary-tree mode in a future revision.
pub const MAX_FILE_SIZE_BYTES: u64 = 28 * 1024 * 1000;

/// Result of chunking a plaintext file.
#[derive(Debug, Clone)]
pub struct ChunkedFile {
    pub attachment_id: [u8; 16],
    pub chunks: Vec<Vec<u8>>,
    pub chunk_hashes: Vec<[u8; 32]>,
    pub merkle_root: [u8; 32],
}

pub struct Chunker;

impl Chunker {
    /// Split a plaintext byte slice into ≤`CHUNK_SIZE_BYTES` chunks, hash each
    /// with SHA-256, and compute the v1 flat-list Merkle root.
    ///
    /// The new `attachment_id` is a fresh v4 UUID — uploaders never reuse one.
    pub fn chunk(bytes: &[u8]) -> Result<ChunkedFile, FilesError> {
        let actual = bytes.len() as u64;
        if actual > MAX_FILE_SIZE_BYTES {
            return Err(FilesError::FileTooLarge {
                actual,
                max: MAX_FILE_SIZE_BYTES,
            });
        }

        let mut chunks: Vec<Vec<u8>> = Vec::new();
        let mut chunk_hashes: Vec<[u8; 32]> = Vec::new();

        for window in bytes.chunks(CHUNK_SIZE_BYTES) {
            let mut hasher = Sha256::new();
            hasher.update(window);
            chunk_hashes.push(hasher.finalize().into());
            chunks.push(window.to_vec());
        }

        // Empty file: one zero-length chunk. Keeps download flow uniform —
        // the receiver still expects a valid Merkle root over a single hash.
        if chunks.is_empty() {
            chunks.push(Vec::new());
            let mut hasher = Sha256::new();
            hasher.update([] as [u8; 0]);
            chunk_hashes.push(hasher.finalize().into());
        }

        let merkle_root = merkle_root_of(&chunk_hashes);

        let attachment_id = *Uuid::new_v4().as_bytes();
        Ok(ChunkedFile {
            attachment_id,
            chunks,
            chunk_hashes,
            merkle_root,
        })
    }
}

/// Construct the v1 flat-list Merkle root.
///
/// `merkle_root = SHA256(chunk_hash_0 || chunk_hash_1 || ... || chunk_hash_n)`.
///
/// This is NOT a binary tree — see plan §1.J6 for why we defer BEP-52.
/// `chunk_hashes.len()` must equal the file's `chunk_count`.
pub fn merkle_root_of(chunk_hashes: &[[u8; 32]]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    for hash in chunk_hashes {
        hasher.update(hash);
    }
    hasher.finalize().into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_reassembles_to_original() {
        let bytes: Vec<u8> = (0..(CHUNK_SIZE_BYTES * 3 + 17))
            .map(|i| (i % 251) as u8)
            .collect();
        let chunked = Chunker::chunk(&bytes).unwrap();
        assert_eq!(chunked.chunks.len(), 4);
        let reassembled: Vec<u8> = chunked.chunks.into_iter().flatten().collect();
        assert_eq!(reassembled, bytes);
    }

    #[test]
    fn chunk_count_math_is_ceil_div() {
        let cases = [
            (0usize, 1usize), // empty file → one zero-length chunk
            (1, 1),
            (CHUNK_SIZE_BYTES, 1),
            (CHUNK_SIZE_BYTES + 1, 2),
            (CHUNK_SIZE_BYTES * 5 - 7, 5),
        ];
        for (size, expected) in cases {
            let bytes = vec![0u8; size];
            let chunked = Chunker::chunk(&bytes).unwrap();
            assert_eq!(chunked.chunks.len(), expected, "size {size}");
            assert_eq!(chunked.chunk_hashes.len(), expected, "size {size}");
        }
    }

    #[test]
    fn merkle_root_is_deterministic() {
        let bytes = b"the quick brown fox jumps over the lazy dog".repeat(1000);
        let a = Chunker::chunk(&bytes).unwrap();
        let b = Chunker::chunk(&bytes).unwrap();
        assert_eq!(a.merkle_root, b.merkle_root);
        // attachment_id should differ — fresh UUID per upload.
        assert_ne!(a.attachment_id, b.attachment_id);
    }

    #[test]
    fn distinct_inputs_have_distinct_roots() {
        let a = Chunker::chunk(b"alpha").unwrap();
        let b = Chunker::chunk(b"beta").unwrap();
        assert_ne!(a.merkle_root, b.merkle_root);
    }

    #[test]
    fn rejects_oversized_files() {
        let oversize = vec![0u8; (MAX_FILE_SIZE_BYTES + 1) as usize];
        let err = Chunker::chunk(&oversize).unwrap_err();
        assert!(matches!(err, FilesError::FileTooLarge { .. }));
    }

    #[test]
    fn empty_file_produces_single_chunk() {
        let chunked = Chunker::chunk(&[]).unwrap();
        assert_eq!(chunked.chunks.len(), 1);
        assert_eq!(chunked.chunks[0].len(), 0);
    }
}
