//! The RFC 9162 §2.1 Merkle tree primitives the directory stands on, with
//! `HASH` = BLAKE3 `derive_key(GOV_DIRECTORY)`.
//!
//! Every function here works on a level of node hashes. Over the leaf
//! hashes (`leaf_hash`, `HASH(0x00 ‖ d)`) [`root`] is §2.1.1's `MTH`, [`path`]
//! is §2.1.3.1's `PATH` and [`consistency`] is §2.1.4.1's `PROOF`. Because
//! RFC 9162 splits `n` at the largest power of two below it, the tree over
//! the 128-leaf page hashes is the top of the same tree, so a leaf's proof
//! is its in-page path followed by its page's path (plan C1).

use std::sync::OnceLock;

use rekindle_types::domains;

/// One node of the directory tree: a leaf hash, an interior node hash or
/// the root.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DirHash(pub [u8; 32]);

/// RFC 9162 §2.1.1 leaf prefix.
const LEAF_PREFIX: u8 = 0x00;
/// RFC 9162 §2.1.1 interior node prefix.
const NODE_PREFIX: u8 = 0x01;

/// A fresh `HASH` state. The derive-key context is hashed once and the
/// keyed state cloned, since a proof hashes hundreds of nodes.
fn hasher() -> blake3::Hasher {
    static BASE: OnceLock<blake3::Hasher> = OnceLock::new();
    BASE.get_or_init(|| blake3::Hasher::new_derive_key(domains::GOV_DIRECTORY))
        .clone()
}

/// `MTH({})` = `HASH()`, the root of an empty directory.
#[must_use]
pub fn empty_root() -> DirHash {
    DirHash(*hasher().finalize().as_bytes())
}

/// `HASH(0x00 ‖ leaf)`.
#[must_use]
pub fn leaf_hash(leaf: &[u8]) -> DirHash {
    let mut h = hasher();
    h.update(&[LEAF_PREFIX]);
    h.update(leaf);
    DirHash(*h.finalize().as_bytes())
}

/// `HASH(0x01 ‖ left ‖ right)`.
#[must_use]
pub fn node_hash(left: &DirHash, right: &DirHash) -> DirHash {
    let mut h = hasher();
    h.update(&[NODE_PREFIX]);
    h.update(&left.0);
    h.update(&right.0);
    DirHash(*h.finalize().as_bytes())
}

/// The largest power of two smaller than `n` (`n >= 2`): RFC 9162's `k`.
fn split(n: usize) -> usize {
    debug_assert!(n >= 2);
    1 << (usize::BITS - 1 - (n - 1).leading_zeros())
}

/// The root of a level of nodes; `MTH({})` for an empty one.
#[must_use]
pub fn root(nodes: &[DirHash]) -> DirHash {
    match nodes.len() {
        0 => empty_root(),
        1 => nodes[0],
        n => {
            let k = split(n);
            node_hash(&root(&nodes[..k]), &root(&nodes[k..]))
        }
    }
}

/// `PATH(m, nodes)`, bottom-up; `m < nodes.len()`.
#[must_use]
pub fn path(m: usize, nodes: &[DirHash]) -> Vec<DirHash> {
    let mut out = Vec::new();
    path_into(m, nodes, &mut out);
    out
}

fn path_into(m: usize, nodes: &[DirHash], out: &mut Vec<DirHash>) {
    let n = nodes.len();
    if n <= 1 {
        return;
    }
    let k = split(n);
    if m < k {
        path_into(m, &nodes[..k], out);
        out.push(root(&nodes[k..]));
    } else {
        path_into(m - k, &nodes[k..], out);
        out.push(root(&nodes[..k]));
    }
}

/// `PROOF(m, nodes)` = `SUBPROOF(m, nodes, true)`; `1 <= m <= nodes.len()`.
#[must_use]
pub fn consistency(m: usize, nodes: &[DirHash]) -> Vec<DirHash> {
    let mut out = Vec::new();
    subproof(m, nodes, true, &mut out);
    out
}

fn subproof(m: usize, nodes: &[DirHash], whole: bool, out: &mut Vec<DirHash>) {
    let n = nodes.len();
    if m == n {
        if !whole {
            out.push(root(nodes));
        }
        return;
    }
    let k = split(n);
    if m <= k {
        subproof(m, &nodes[..k], whole, out);
        out.push(root(&nodes[k..]));
    } else {
        subproof(m - k, &nodes[k..], false, out);
        out.push(root(&nodes[..k]));
    }
}

/// RFC 9162 §2.1.3.2: does `path` lead node `start`, at `index` of a level
/// of `size` nodes, to `expected`?
#[must_use]
pub fn verify_path(
    index: u64,
    size: u64,
    start: DirHash,
    path: &[DirHash],
    expected: &DirHash,
) -> bool {
    if index >= size {
        return false;
    }
    let (mut fn_, mut sn) = (index, size - 1);
    let mut r = start;
    for p in path {
        if sn == 0 {
            return false;
        }
        if fn_ & 1 == 1 || fn_ == sn {
            r = node_hash(p, &r);
            while fn_ & 1 == 0 && fn_ != 0 {
                fn_ >>= 1;
                sn >>= 1;
            }
        } else {
            r = node_hash(&r, p);
        }
        fn_ >>= 1;
        sn >>= 1;
    }
    sn == 0 && r == *expected
}

/// RFC 9162 §2.1.4.2: does `proof` show the tree of `first` leaves with
/// root `first_root` is a prefix of the tree of `second` leaves with root
/// `second_root`? `first == second` holds with an empty proof and equal
/// roots; `first == 0` proves nothing and is refused.
#[must_use]
pub fn verify_consistency(
    first: u64,
    second: u64,
    first_root: &DirHash,
    second_root: &DirHash,
    proof: &[DirHash],
) -> bool {
    if first == 0 || first > second {
        return false;
    }
    if first == second {
        return proof.is_empty() && first_root == second_root;
    }
    let mut path: Vec<DirHash> = Vec::with_capacity(proof.len() + 1);
    if first.is_power_of_two() {
        path.push(*first_root);
    }
    path.extend_from_slice(proof);
    let Some((seed, rest)) = path.split_first() else {
        return false;
    };
    let (mut fn_, mut sn) = (first - 1, second - 1);
    while fn_ & 1 == 1 {
        fn_ >>= 1;
        sn >>= 1;
    }
    let (mut fr, mut sr) = (*seed, *seed);
    for c in rest {
        if sn == 0 {
            return false;
        }
        if fn_ & 1 == 1 || fn_ == sn {
            fr = node_hash(c, &fr);
            sr = node_hash(c, &sr);
            while fn_ & 1 == 0 && fn_ != 0 {
                fn_ >>= 1;
                sn >>= 1;
            }
        } else {
            sr = node_hash(&sr, c);
        }
        fn_ >>= 1;
        sn >>= 1;
    }
    fr == *first_root && sr == *second_root && sn == 0
}
