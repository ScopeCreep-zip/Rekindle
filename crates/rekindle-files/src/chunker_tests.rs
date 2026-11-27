#![allow(
    clippy::cast_possible_truncation,
    reason = "test fixture: the constants are compile-time known and well inside range; a fallible conversion here would add noise without adding safety"
)]

use super::*;

#[test]
fn chunk_reassembles_to_original() {
    let bytes: Vec<u8> = (0..(CHUNK_SIZE_BYTES * 3 + 17))
        .map(|i| (i % 251) as u8)
        .collect();
    let chunked = Chunker::chunk(&bytes, [1; 16]).unwrap();
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
        let chunked = Chunker::chunk(&bytes, [1; 16]).unwrap();
        assert_eq!(chunked.chunks.len(), expected, "size {size}");
        assert_eq!(chunked.chunk_hashes.len(), expected, "size {size}");
    }
}

#[test]
fn merkle_root_is_deterministic() {
    let bytes = b"the quick brown fox jumps over the lazy dog".repeat(1000);
    let a = Chunker::chunk(&bytes, [1; 16]).unwrap();
    let b = Chunker::chunk(&bytes, [2; 16]).unwrap();
    assert_eq!(a.merkle_root, b.merkle_root);
    // attachment_id is the uploader's fresh id, whatever the bytes.
    assert_eq!((a.attachment_id, b.attachment_id), ([1; 16], [2; 16]));
}

#[test]
fn distinct_inputs_have_distinct_roots() {
    let a = Chunker::chunk(b"alpha", [1; 16]).unwrap();
    let b = Chunker::chunk(b"beta", [1; 16]).unwrap();
    assert_ne!(a.merkle_root, b.merkle_root);
}

#[test]
fn rejects_oversized_files() {
    let oversize = vec![0u8; (MAX_FILE_SIZE_BYTES + 1) as usize];
    let err = Chunker::chunk(&oversize, [1; 16]).unwrap_err();
    assert!(matches!(err, FilesError::FileTooLarge { .. }));
}

#[test]
fn empty_file_produces_single_chunk() {
    let chunked = Chunker::chunk(&[], [1; 16]).unwrap();
    assert_eq!(chunked.chunks.len(), 1);
    assert_eq!(chunked.chunks[0].len(), 0);
}
