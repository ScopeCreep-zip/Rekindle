//! The checkpoint directory (plan E3.2 S7c.2): every member a checkpoint's
//! cut admitted, as `DirLeafV1` leaves of one RFC 9162 §2.1.1 Merkle tree,
//! served in 128-leaf pages.
//!
//! The directory is append-only along the `prevCheckpoint` chain: the
//! previous checkpoint's leaves first, in their order, then each member
//! first admitted in this cut by its `MemberJoined`'s `(lamport, op_hash)`.
//! A removal never edits a leaf (it is in `stateRoot`'s `removed` and
//! `bans`, S7c.3), so RFC 9162 §2.1.4 consistency proofs hold between two
//! checkpoints. A page of 128 leaves starting at a multiple of 128 is a
//! complete subtree of the one tree (plan C1), so an entry's proof is up to
//! 7 in-page hashes plus the page-to-root path. A leaf carries the member's
//! record key and its owner (C2): pages are Discovery, and a joiner
//! prefetches member records from them.

mod tree;

pub use tree::{empty_root, leaf_hash, node_hash, DirHash};

use rekindle_types::governance::OpHash;
use rekindle_types::id::PseudonymKey;

/// `DirLeafV1.version`.
pub const DIR_LEAF_VERSION: u8 = 1;
/// `DirLeafV1.kind` of a member leaf, the only kind.
pub const DIR_LEAF_KIND_MEMBER: u8 = 1;
/// An encoded `DirLeafV1`: 1 + 1 + 32 + 32 + 68 + 32 + 32 + 8 bytes.
pub const DIR_LEAF_LEN: usize = 206;
/// Leaves per directory page: a power of two, so a page is a complete
/// subtree of the directory tree (plan C1).
pub const DIR_PAGE_LEAVES: usize = 128;
/// `DirectoryPage.version`.
pub const DIR_PAGE_VERSION: u8 = 1;
/// A page header: version 1, `community_id` 32, `checkpoint_height` 8,
/// `first_index` 8, `count` 2.
pub const DIR_PAGE_HEADER_LEN: usize = 51;
/// The longest page-to-root path a page carries: 2^16 pages, 2^23 leaves.
pub const DIR_PAGE_MAX_PATH: usize = 16;

/// The binary typed key of a DFLT(8) member record: the crypto kind, the
/// record key and the encryption key Veilid always creates with it
/// (veilid-core `storage_manager/create_record.rs:86`, `BareRecordKey`).
/// Without the encryption key a reader cannot open `M(p)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MemberRecordKey {
    pub kind: [u8; 4],
    pub key: [u8; 32],
    pub encryption_key: [u8; 32],
}

/// One directory entry. Encoded big-endian in field order, 206 bytes.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DirLeafV1 {
    pub community_id: [u8; 32],
    pub member: PseudonymKey,
    pub member_record: MemberRecordKey,
    /// The member record's owner public key (`K_M`).
    pub member_record_owner: [u8; 32],
    /// The member's `MemberJoined`.
    pub join_op_hash: OpHash,
    /// The checkpoint height whose cut first included the member.
    pub join_height: u64,
}

/// Why directory bytes or a page were refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirError {
    Length,
    Version,
    Kind,
    /// `count` is 0 or above 128, or not the page's share of `dir_size`.
    Count,
    /// `first_index` is not a page boundary inside the directory.
    FirstIndex,
    CommunityMismatch,
    HeightMismatch,
    /// The page-to-root path is too long or does not reach `dir_root`.
    Path,
}

fn take<const N: usize>(bytes: &[u8], at: &mut usize) -> [u8; N] {
    let mut out = [0u8; N];
    out.copy_from_slice(&bytes[*at..*at + N]);
    *at += N;
    out
}

impl DirLeafV1 {
    #[must_use]
    pub fn encode(&self) -> [u8; DIR_LEAF_LEN] {
        let mut out = [0u8; DIR_LEAF_LEN];
        let parts: [&[u8]; 9] = [
            &[DIR_LEAF_VERSION, DIR_LEAF_KIND_MEMBER],
            &self.community_id,
            &self.member.0,
            &self.member_record.kind,
            &self.member_record.key,
            &self.member_record.encryption_key,
            &self.member_record_owner,
            &self.join_op_hash.0,
            &self.join_height.to_be_bytes(),
        ];
        let mut at = 0;
        for part in parts {
            out[at..at + part.len()].copy_from_slice(part);
            at += part.len();
        }
        out
    }

    /// # Errors
    /// [`DirError::Length`], [`DirError::Version`] or [`DirError::Kind`].
    pub fn decode(bytes: &[u8]) -> Result<Self, DirError> {
        if bytes.len() != DIR_LEAF_LEN {
            return Err(DirError::Length);
        }
        if bytes[0] != DIR_LEAF_VERSION {
            return Err(DirError::Version);
        }
        if bytes[1] != DIR_LEAF_KIND_MEMBER {
            return Err(DirError::Kind);
        }
        let mut at = 2;
        Ok(Self {
            community_id: take(bytes, &mut at),
            member: PseudonymKey(take(bytes, &mut at)),
            member_record: MemberRecordKey {
                kind: take(bytes, &mut at),
                key: take(bytes, &mut at),
                encryption_key: take(bytes, &mut at),
            },
            member_record_owner: take(bytes, &mut at),
            join_op_hash: OpHash(take(bytes, &mut at)),
            join_height: u64::from_be_bytes(take(bytes, &mut at)),
        })
    }

    /// `HASH(0x00 ‖ encode())`.
    #[must_use]
    pub fn hash(&self) -> DirHash {
        leaf_hash(&self.encode())
    }
}

/// RFC 9162 §2.1.3 inclusion proof of the leaf at `leaf_index` in the
/// directory of `tree_size` leaves, bottom-up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InclusionProofV1 {
    pub leaf_index: u64,
    pub tree_size: u64,
    pub path: Vec<DirHash>,
}

impl InclusionProofV1 {
    /// Is `leaf` at `leaf_index` of the directory with root `dir_root`?
    /// `tree_size` must be the endorsed checkpoint's `dirSize`: RFC 9162
    /// binds the size by the signed tree head, not by the path.
    #[must_use]
    pub fn verify(&self, leaf: &DirLeafV1, dir_root: &DirHash) -> bool {
        tree::verify_path(
            self.leaf_index,
            self.tree_size,
            leaf.hash(),
            &self.path,
            dir_root,
        )
    }
}

/// RFC 9162 §2.1.4 consistency proof: the directory of `first_size` leaves
/// is a prefix of the one of `second_size`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsistencyProofV1 {
    pub first_size: u64,
    pub second_size: u64,
    pub path: Vec<DirHash>,
}

impl ConsistencyProofV1 {
    #[must_use]
    pub fn verify(&self, first_root: &DirHash, second_root: &DirHash) -> bool {
        tree::verify_consistency(
            self.first_size,
            self.second_size,
            first_root,
            second_root,
            &self.path,
        )
    }
}

/// A checkpoint's directory: its leaves in order, with their hashes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CheckpointDirectory {
    leaves: Vec<DirLeafV1>,
    hashes: Vec<DirHash>,
    /// Each page's subtree hash, the leaves of the tree's top.
    pages: Vec<DirHash>,
}

fn to_u64(n: usize) -> u64 {
    u64::try_from(n).unwrap_or(u64::MAX)
}

impl CheckpointDirectory {
    #[must_use]
    pub fn new(leaves: Vec<DirLeafV1>) -> Self {
        let hashes: Vec<DirHash> = leaves.iter().map(DirLeafV1::hash).collect();
        let pages = hashes.chunks(DIR_PAGE_LEAVES).map(tree::root).collect();
        Self {
            leaves,
            hashes,
            pages,
        }
    }

    /// Append a member first admitted in this cut.
    pub fn push(&mut self, leaf: DirLeafV1) {
        self.hashes.push(leaf.hash());
        self.leaves.push(leaf);
        let last = (self.hashes.len() - 1) / DIR_PAGE_LEAVES;
        let page = tree::root(&self.hashes[last * DIR_PAGE_LEAVES..]);
        self.pages.truncate(last);
        self.pages.push(page);
    }

    #[must_use]
    pub fn leaves(&self) -> &[DirLeafV1] {
        &self.leaves
    }

    #[must_use]
    pub fn size(&self) -> u64 {
        to_u64(self.leaves.len())
    }

    /// `dirRoot`: `MTH` over the leaves.
    #[must_use]
    pub fn root(&self) -> DirHash {
        tree::root(&self.hashes)
    }

    #[must_use]
    pub fn page_count(&self) -> usize {
        self.leaves.len().div_ceil(DIR_PAGE_LEAVES)
    }

    /// Page `page` as the checkpoint at `checkpoint_height` publishes it.
    #[must_use]
    pub fn page(
        &self,
        community_id: [u8; 32],
        checkpoint_height: u64,
        page: usize,
    ) -> Option<DirectoryPage> {
        let first = page.checked_mul(DIR_PAGE_LEAVES)?;
        if first >= self.leaves.len() {
            return None;
        }
        let end = (first + DIR_PAGE_LEAVES).min(self.leaves.len());
        Some(DirectoryPage {
            community_id,
            checkpoint_height,
            first_index: to_u64(first),
            leaves: self.leaves[first..end].to_vec(),
            path: tree::path(page, &self.pages),
        })
    }

    /// The proof of the leaf at `index`: its in-page path, then its page's
    /// path to the root (the same list as RFC 9162's `PATH`).
    #[must_use]
    pub fn inclusion_proof(&self, index: usize) -> Option<InclusionProofV1> {
        if index >= self.leaves.len() {
            return None;
        }
        let page = index / DIR_PAGE_LEAVES;
        let first = page * DIR_PAGE_LEAVES;
        let end = (first + DIR_PAGE_LEAVES).min(self.leaves.len());
        let mut path = tree::path(index - first, &self.hashes[first..end]);
        path.extend(tree::path(page, &self.pages));
        Some(InclusionProofV1 {
            leaf_index: to_u64(index),
            tree_size: self.size(),
            path,
        })
    }

    /// The proof that this directory's first `first_size` leaves were the
    /// whole directory of an earlier checkpoint; `1 <= first_size <= size`.
    #[must_use]
    pub fn consistency_proof(&self, first_size: usize) -> Option<ConsistencyProofV1> {
        if first_size == 0 || first_size > self.leaves.len() {
            return None;
        }
        Some(ConsistencyProofV1 {
            first_size: to_u64(first_size),
            second_size: self.size(),
            path: tree::consistency(first_size, &self.hashes),
        })
    }
}

/// One directory page, a sealed DFLT(1) value (ADR 0011 "Sealed pages"):
/// a 51-byte header, `count` leaves and the page-to-root path. Nothing on
/// it is signed: it is proven by its path to the endorsed `dirRoot`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectoryPage {
    pub community_id: [u8; 32],
    pub checkpoint_height: u64,
    /// The page number × 128.
    pub first_index: u64,
    /// 1–128 leaves; below 128 only on the last page.
    pub leaves: Vec<DirLeafV1>,
    pub path: Vec<DirHash>,
}

impl DirectoryPage {
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let count = u16::try_from(self.leaves.len()).unwrap_or(u16::MAX);
        let mut out = Vec::with_capacity(
            DIR_PAGE_HEADER_LEN + self.leaves.len() * DIR_LEAF_LEN + self.path.len() * 32,
        );
        out.push(DIR_PAGE_VERSION);
        out.extend_from_slice(&self.community_id);
        out.extend_from_slice(&self.checkpoint_height.to_be_bytes());
        out.extend_from_slice(&self.first_index.to_be_bytes());
        out.extend_from_slice(&count.to_be_bytes());
        for leaf in &self.leaves {
            out.extend_from_slice(&leaf.encode());
        }
        for hash in &self.path {
            out.extend_from_slice(&hash.0);
        }
        out
    }

    /// The path is what follows the leaves, in 32-byte hashes.
    ///
    /// # Errors
    /// A malformed header, count, leaf or path.
    pub fn decode(bytes: &[u8]) -> Result<Self, DirError> {
        if bytes.len() < DIR_PAGE_HEADER_LEN {
            return Err(DirError::Length);
        }
        if bytes[0] != DIR_PAGE_VERSION {
            return Err(DirError::Version);
        }
        let mut at = 1;
        let community_id = take(bytes, &mut at);
        let checkpoint_height = u64::from_be_bytes(take(bytes, &mut at));
        let first_index = u64::from_be_bytes(take(bytes, &mut at));
        let count = usize::from(u16::from_be_bytes(take(bytes, &mut at)));
        if count == 0 || count > DIR_PAGE_LEAVES {
            return Err(DirError::Count);
        }
        let leaves_end = at + count * DIR_LEAF_LEN;
        let rest = bytes
            .len()
            .checked_sub(leaves_end)
            .ok_or(DirError::Length)?;
        if !rest.is_multiple_of(32) {
            return Err(DirError::Length);
        }
        if rest / 32 > DIR_PAGE_MAX_PATH {
            return Err(DirError::Path);
        }
        let leaves = bytes[at..leaves_end]
            .chunks(DIR_LEAF_LEN)
            .map(DirLeafV1::decode)
            .collect::<Result<Vec<_>, _>>()?;
        let mut at = leaves_end;
        let path = (0..rest / 32)
            .map(|_| DirHash(take(bytes, &mut at)))
            .collect();
        Ok(Self {
            community_id,
            checkpoint_height,
            first_index,
            leaves,
            path,
        })
    }

    /// Check the page against the checkpoint that names it: its community,
    /// its height, a page boundary inside `dir_size`, the page's full
    /// share of leaves, every leaf in the community, and the page subtree's
    /// path to `dir_root`.
    ///
    /// # Errors
    /// The first [`DirError`] the page fails.
    pub fn verify(
        &self,
        community_id: &[u8; 32],
        checkpoint_height: u64,
        dir_size: u64,
        dir_root: &DirHash,
    ) -> Result<(), DirError> {
        if self.community_id != *community_id
            || self.leaves.iter().any(|l| l.community_id != *community_id)
        {
            return Err(DirError::CommunityMismatch);
        }
        if self.checkpoint_height != checkpoint_height {
            return Err(DirError::HeightMismatch);
        }
        let per_page = to_u64(DIR_PAGE_LEAVES);
        if !self.first_index.is_multiple_of(per_page) || self.first_index >= dir_size {
            return Err(DirError::FirstIndex);
        }
        if to_u64(self.leaves.len()) != (dir_size - self.first_index).min(per_page) {
            return Err(DirError::Count);
        }
        if self.path.len() > DIR_PAGE_MAX_PATH {
            return Err(DirError::Path);
        }
        let hashes: Vec<DirHash> = self.leaves.iter().map(DirLeafV1::hash).collect();
        let page_index = self.first_index / per_page;
        let pages = dir_size.div_ceil(per_page);
        if tree::verify_path(page_index, pages, tree::root(&hashes), &self.path, dir_root) {
            Ok(())
        } else {
            Err(DirError::Path)
        }
    }

    /// The full inclusion proof of the page's `offset`th leaf in the
    /// directory of `dir_size` leaves: its in-page path, then the page's.
    #[must_use]
    pub fn inclusion_proof(&self, offset: usize, dir_size: u64) -> Option<InclusionProofV1> {
        if offset >= self.leaves.len() {
            return None;
        }
        let hashes: Vec<DirHash> = self.leaves.iter().map(DirLeafV1::hash).collect();
        let mut path = tree::path(offset, &hashes);
        path.extend_from_slice(&self.path);
        Some(InclusionProofV1 {
            leaf_index: self.first_index + to_u64(offset),
            tree_size: dir_size,
            path,
        })
    }
}

#[cfg(test)]
mod tests;
