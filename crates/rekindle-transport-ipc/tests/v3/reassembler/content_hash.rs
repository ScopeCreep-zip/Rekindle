use rekindle_transport_ipc::v3::io::lane_channels::PlaintextBuf;
use rekindle_transport_ipc::v3::stream::reassembler::Reassembler;

fn ins(r: &mut Reassembler, idx: u32, data: &[u8]) -> Vec<(u32, PlaintextBuf)> {
    let digest = *blake3::hash(data).as_bytes();
    r.insert_with_digest(idx, PlaintextBuf::Owned(data.to_vec()), digest)
}

#[test]
fn content_hash_verified_on_fin() {
    let mut r = Reassembler::new(256);
    let data = b"the quick brown fox";
    let mut h = blake3::Hasher::new();
    h.update(blake3::hash(data).as_bytes());
    let expected_hash = h.finalize();

    ins(&mut r, 0, data);
    let result = r.verify_content_hash(expected_hash.as_bytes());
    assert!(result.is_ok(), "correct hash must verify");
}

#[test]
fn content_hash_mismatch_on_fin() {
    let mut r = Reassembler::new(256);
    ins(&mut r, 0, b"actual data");

    let wrong_hash = [0xFF; 32];
    let result = r.verify_content_hash(&wrong_hash);
    assert!(result.is_err(), "wrong hash must fail");
}

#[test]
fn zero_content_hash_skips_verification() {
    let mut r = Reassembler::new(256);
    ins(&mut r, 0, b"anything");

    let zero_hash = [0u8; 32];
    let result = r.verify_content_hash(&zero_hash);
    assert!(result.is_ok(), "all-zero hash must skip verification");
}

#[test]
fn hash_computed_incrementally_matches_whole() {
    let chunk_a = vec![0xAA; 1024];
    let chunk_b = vec![0xBB; 1024];
    let chunk_c = vec![0xCC; 1024];

    let mut h = blake3::Hasher::new();
    h.update(blake3::hash(&chunk_a).as_bytes());
    h.update(blake3::hash(&chunk_b).as_bytes());
    h.update(blake3::hash(&chunk_c).as_bytes());
    let expected = h.finalize();

    let mut r = Reassembler::new(256);
    ins(&mut r, 0, &chunk_a);
    ins(&mut r, 1, &chunk_b);
    ins(&mut r, 2, &chunk_c);

    let result = r.verify_content_hash(expected.as_bytes());
    assert!(result.is_ok(), "Merkle hash must match reassembler digest");
}

#[test]
fn hash_of_out_of_order_chunks_matches() {
    let chunk_0 = vec![0x00; 512];
    let chunk_1 = vec![0x11; 512];
    let chunk_2 = vec![0x22; 512];

    let mut h = blake3::Hasher::new();
    h.update(blake3::hash(&chunk_0).as_bytes());
    h.update(blake3::hash(&chunk_1).as_bytes());
    h.update(blake3::hash(&chunk_2).as_bytes());
    let expected = h.finalize();

    let mut r = Reassembler::new(256);
    ins(&mut r, 2, &chunk_2);
    ins(&mut r, 0, &chunk_0);
    ins(&mut r, 1, &chunk_1);

    let result = r.verify_content_hash(expected.as_bytes());
    assert!(result.is_ok(), "reordered chunks must produce same Merkle hash");
}
