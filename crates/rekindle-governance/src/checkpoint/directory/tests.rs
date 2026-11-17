//! CheckpointDirectory tests: the tree against a literal transcription of RFC 9162
//! §2.1's `MTH`, `PATH` and `PROOF`, every inclusion proof up to 300
//! leaves, every consistency proof up to 260, the page boundaries at 128
//! and 256, and the plan's named tests.

use super::tree::{self, DirHash};
use super::*;
use rekindle_types::dht_layout::MAX_SUBKEY_SIZE;
use rekindle_types::domains;

const DIR_COMMUNITY: [u8; 32] = [0xC0; 32];

fn h(parts: &[&[u8]]) -> DirHash {
    let mut hasher = blake3::Hasher::new_derive_key(domains::GOV_DIRECTORY);
    for p in parts {
        hasher.update(p);
    }
    DirHash(*hasher.finalize().as_bytes())
}

fn k_of(n: usize) -> usize {
    let mut k = 1;
    while k * 2 < n {
        k *= 2;
    }
    k
}

/// RFC 9162 §2.1.1 `MTH`, transcribed.
fn mth(d: &[Vec<u8>]) -> DirHash {
    match d.len() {
        0 => h(&[]),
        1 => h(&[&[0x00], &d[0]]),
        n => {
            let k = k_of(n);
            h(&[&[0x01], &mth(&d[..k]).0, &mth(&d[k..]).0])
        }
    }
}

/// RFC 9162 §2.1.3.1 `PATH`, transcribed.
fn rfc_path(m: usize, d: &[Vec<u8>]) -> Vec<DirHash> {
    let n = d.len();
    if n <= 1 {
        return Vec::new();
    }
    let k = k_of(n);
    let mut out;
    if m < k {
        out = rfc_path(m, &d[..k]);
        out.push(mth(&d[k..]));
    } else {
        out = rfc_path(m - k, &d[k..]);
        out.push(mth(&d[..k]));
    }
    out
}

/// RFC 9162 §2.1.4.1 `SUBPROOF`, transcribed.
fn rfc_subproof(m: usize, d: &[Vec<u8>], whole: bool) -> Vec<DirHash> {
    let n = d.len();
    if m == n {
        return if whole { Vec::new() } else { vec![mth(d)] };
    }
    let k = k_of(n);
    let mut out;
    if m <= k {
        out = rfc_subproof(m, &d[..k], whole);
        out.push(mth(&d[k..]));
    } else {
        out = rfc_subproof(m - k, &d[k..], false);
        out.push(mth(&d[..k]));
    }
    out
}

fn leaf(i: u64) -> DirLeafV1 {
    let b = |tag: u8| {
        let mut out = [tag; 32];
        out[..8].copy_from_slice(&i.to_be_bytes());
        out
    };
    DirLeafV1 {
        community_id: DIR_COMMUNITY,
        member: PseudonymKey(b(1)),
        member_record: MemberRecordKey {
            kind: *b"VLD0",
            key: b(2),
            encryption_key: b(3),
        },
        member_record_owner: b(4),
        join_op_hash: OpHash(b(5)),
        join_height: i / 7 + 1,
    }
}

fn directory(n: u64) -> CheckpointDirectory {
    CheckpointDirectory::new((0..n).map(leaf).collect())
}

fn encoded(n: u64) -> Vec<Vec<u8>> {
    (0..n).map(|i| leaf(i).encode().to_vec()).collect()
}

#[test]
fn the_root_is_rfc_9162_mth() {
    assert_eq!(CheckpointDirectory::default().root(), mth(&[]));
    for n in [1, 2, 3, 5, 127, 128, 129, 255, 256, 257, 300] {
        assert_eq!(directory(n).root(), mth(&encoded(n)), "n = {n}");
    }
}

#[test]
fn a_leaf_hash_is_never_a_node_hash() {
    // Without CT's 0x00/0x01 prefixes, a 64-byte leaf equal to two child
    // hashes would hash to their parent (RFC 9162 §2.1.1's second
    // preimage).
    let (a, b) = (tree::leaf_hash(b"a"), tree::leaf_hash(b"b"));
    let joined = [a.0, b.0].concat();
    assert_ne!(tree::leaf_hash(&joined), tree::node_hash(&a, &b));
}

#[test]
fn every_inclusion_proof_verifies_up_to_300_leaves() {
    for n in 1..=300u64 {
        let dir = directory(n);
        let root = dir.root();
        let d = [1, 2, 3, 5, 127, 128, 129, 255, 256, 257, 300]
            .contains(&n)
            .then(|| encoded(n));
        for i in 0..n {
            let idx = usize::try_from(i).unwrap();
            let proof = dir.inclusion_proof(idx).unwrap();
            assert!(proof.verify(&leaf(i), &root), "n = {n}, i = {i}");
            if let Some(d) = &d {
                assert_eq!(proof.path, rfc_path(idx, d), "n = {n}, i = {i}");
            }
        }
        assert!(dir.inclusion_proof(usize::try_from(n).unwrap()).is_none());
    }
}

#[test]
fn every_consistency_proof_verifies_up_to_260_leaves() {
    let all = directory(260);
    let roots: Vec<DirHash> = (0..=260).map(|n| directory(n).root()).collect();
    for n in 1..=260usize {
        let dir = CheckpointDirectory::new(all.leaves()[..n].to_vec());
        let d = [1, 2, 3, 127, 128, 129, 255, 256, 257, 260]
            .contains(&n)
            .then(|| encoded(n as u64));
        for m in 1..=n {
            let proof = dir.consistency_proof(m).unwrap();
            assert!(proof.verify(&roots[m], &roots[n]), "m = {m}, n = {n}");
            if let Some(d) = &d {
                assert_eq!(proof.path, rfc_subproof(m, d, true), "m = {m}, n = {n}");
            }
            if m < n {
                assert!(!proof.verify(&roots[m - 1], &roots[n]), "m = {m}, n = {n}");
            }
        }
        assert!(dir.consistency_proof(0).is_none());
    }
}

#[test]
fn a_tampered_leaf_or_proof_fails() {
    let dir = directory(200);
    let root = dir.root();
    let proof = dir.inclusion_proof(150).unwrap();
    let mut other = leaf(150);
    other.member_record_owner[31] ^= 1;
    assert!(!proof.verify(&other, &root));
    assert!(!proof.verify(&leaf(149), &root));
    for at in 0..proof.path.len() {
        let mut bad = proof.clone();
        bad.path[at].0[0] ^= 1;
        assert!(!bad.verify(&leaf(150), &root), "hash {at}");
    }
    let mut short = proof.clone();
    short.path.pop();
    assert!(!short.verify(&leaf(150), &root));
    let mut long = proof.clone();
    long.path.push(root);
    assert!(!long.verify(&leaf(150), &root));
    let mut moved = proof.clone();
    moved.leaf_index = 151;
    assert!(!moved.verify(&leaf(150), &root));
    let mut resized = proof;
    resized.tree_size = 151;
    assert!(!resized.verify(&leaf(150), &root));

    let first = directory(130).root();
    let mut cons = dir.consistency_proof(130).unwrap();
    assert!(cons.verify(&first, &root));
    cons.path[0].0[5] ^= 1;
    assert!(!cons.verify(&first, &root));
}

#[test]
fn a_reordered_directory_is_not_consistent() {
    // Append-only: inserting a late join before older leaves breaks the
    // proof against the earlier checkpoint.
    let before = directory(129);
    let mut leaves = before.leaves().to_vec();
    leaves.insert(5, leaf(999));
    let after = CheckpointDirectory::new(leaves);
    let proof = after.consistency_proof(129).unwrap();
    assert!(!proof.verify(&before.root(), &after.root()));
}

#[test]
fn a_page_proves_against_dir_root() {
    for n in [1u64, 127, 128, 129, 255, 256, 257, 300] {
        let dir = directory(n);
        let root = dir.root();
        for p in 0..dir.page_count() {
            let page = dir.page(DIR_COMMUNITY, 3, p).unwrap();
            let page = DirectoryPage::decode(&page.encode()).unwrap();
            assert_eq!(
                page.verify(&DIR_COMMUNITY, 3, n, &root),
                Ok(()),
                "n = {n}, page {p}"
            );
            assert!(page.leaves.len() <= DIR_PAGE_LEAVES);
            for (offset, l) in page.leaves.iter().enumerate() {
                let proof = page.inclusion_proof(offset, n).unwrap();
                let index = usize::try_from(proof.leaf_index).unwrap();
                assert_eq!(Some(&proof), dir.inclusion_proof(index).as_ref());
                assert!(proof.path.len() - page.path.len() <= 7);
                assert!(proof.verify(l, &root));
            }
            assert!(page.verify(&DIR_COMMUNITY, 4, n, &root).is_err());
            assert!(page.verify(&[0; 32], 3, n, &root).is_err());
        }
        assert!(dir.page(DIR_COMMUNITY, 3, dir.page_count()).is_none());
    }
}

#[test]
fn a_page_with_a_tampered_leaf_or_path_fails() {
    let dir = directory(300);
    let root = dir.root();
    let page = dir.page(DIR_COMMUNITY, 2, 1).unwrap();
    let mut bad = page.clone();
    bad.leaves[0].member_record.encryption_key[0] ^= 1;
    assert_eq!(
        bad.verify(&DIR_COMMUNITY, 2, 300, &root),
        Err(DirError::Path)
    );
    let mut bad = page.clone();
    bad.path[0].0[0] ^= 1;
    assert_eq!(
        bad.verify(&DIR_COMMUNITY, 2, 300, &root),
        Err(DirError::Path)
    );
    let mut bad = page.clone();
    bad.leaves.pop();
    assert_eq!(
        bad.verify(&DIR_COMMUNITY, 2, 300, &root),
        Err(DirError::Count)
    );
    let mut bad = page;
    bad.first_index = 129;
    assert_eq!(
        bad.verify(&DIR_COMMUNITY, 2, 300, &root),
        Err(DirError::FirstIndex)
    );
    let mut grown = dir.page(DIR_COMMUNITY, 2, 2).unwrap();
    assert_eq!(
        grown.verify(&DIR_COMMUNITY, 2, 299, &root),
        Err(DirError::Count)
    );
    grown.leaves.push(leaf(300));
    assert_eq!(
        grown.verify(&DIR_COMMUNITY, 2, 301, &root),
        Err(DirError::Path)
    );
}

#[test]
fn the_page_boundaries_are_complete_subtrees() {
    // A full page's hash is MTH of its 128 leaves alone, and the directory
    // of 256 is the node over its two pages.
    let d = encoded(257);
    let dir = directory(257);
    let pages: Vec<DirHash> = d.chunks(128).map(mth).collect();
    assert_eq!(directory(256).root(), tree::node_hash(&pages[0], &pages[1]));
    let top = tree::node_hash(&pages[0], &pages[1]);
    assert_eq!(dir.root(), tree::node_hash(&top, &pages[2]));
    assert_eq!(dir.page(DIR_COMMUNITY, 1, 2).unwrap().path, vec![top]);
    assert_eq!(
        dir.page(DIR_COMMUNITY, 1, 0).unwrap().path,
        vec![pages[1], pages[2]]
    );
    assert!(directory(128)
        .page(DIR_COMMUNITY, 1, 0)
        .unwrap()
        .path
        .is_empty());
}

#[test]
fn consistency_holds_between_two_checkpoints() {
    // Checkpoint 1 admits 130 members; checkpoint 2 keeps them in order
    // and appends the 70 its cut first admitted.
    let mut dir = directory(130);
    let first = dir.root();
    for i in 130..200 {
        dir.push(leaf(i));
    }
    let proof = dir.consistency_proof(130).unwrap();
    assert!(proof.verify(&first, &dir.root()));
    assert_eq!(dir, directory(200));
}

#[test]
fn a_full_page_fits_one_dht_value() {
    let page = DirectoryPage {
        community_id: DIR_COMMUNITY,
        checkpoint_height: u64::MAX,
        first_index: 0,
        leaves: (0..128).map(leaf).collect(),
        path: vec![DirHash([7; 32]); DIR_PAGE_MAX_PATH],
    };
    let bytes = page.encode();
    assert_eq!(leaf(0).encode().len(), 206);
    assert_eq!(bytes.len(), 51 + 128 * 206 + 16 * 32);
    assert!(bytes.len() <= MAX_SUBKEY_SIZE);
    assert_eq!(DirectoryPage::decode(&bytes), Ok(page));
}

#[test]
fn a_page_names_each_members_record_and_owner() {
    let dir = directory(140);
    let page = DirectoryPage::decode(&dir.page(DIR_COMMUNITY, 1, 1).unwrap().encode()).unwrap();
    assert_eq!(page.first_index, 128);
    for (offset, l) in page.leaves.iter().enumerate() {
        let want = leaf(128 + offset as u64);
        assert_eq!(l.member, want.member);
        assert_eq!(l.member_record, want.member_record);
        assert_eq!(
            l.member_record.encryption_key,
            want.member_record.encryption_key
        );
        assert_eq!(l.member_record_owner, want.member_record_owner);
    }
}

#[test]
fn malformed_bytes_are_refused() {
    let bytes = leaf(1).encode();
    assert_eq!(DirLeafV1::decode(&bytes), Ok(leaf(1)));
    assert_eq!(DirLeafV1::decode(&bytes[..205]), Err(DirError::Length));
    let mut bad = bytes;
    bad[0] = 2;
    assert_eq!(DirLeafV1::decode(&bad), Err(DirError::Version));
    let mut bad = bytes;
    bad[1] = 2;
    assert_eq!(DirLeafV1::decode(&bad), Err(DirError::Kind));

    let page = directory(3).page(DIR_COMMUNITY, 1, 0).unwrap().encode();
    assert_eq!(DirectoryPage::decode(&page[..50]), Err(DirError::Length));
    assert_eq!(
        DirectoryPage::decode(&page[..page.len() - 1]),
        Err(DirError::Length)
    );
    let mut bad = page.clone();
    bad[49..51].copy_from_slice(&0u16.to_be_bytes());
    assert_eq!(DirectoryPage::decode(&bad), Err(DirError::Count));
    bad[49..51].copy_from_slice(&129u16.to_be_bytes());
    assert_eq!(DirectoryPage::decode(&bad), Err(DirError::Count));
    let mut long = page;
    long.extend(std::iter::repeat_n(0u8, 17 * 32));
    assert_eq!(DirectoryPage::decode(&long), Err(DirError::Path));
}
