//! A checkpoint op's digest and its op-local checks on ingest (plan E3.2
//! S7c.3).

use rekindle_types::dht_layout::member_record::MAX_FRONTIER_REFS;
use rekindle_types::domains;
use rekindle_types::governance::{Checkpoint, GovernanceEntry, GovernanceOp, OpHash};

use crate::op::OpRejection;

/// Most heads a cut names: every one is a dep, beside the author's `prev`,
/// within the op's `MAX_FRONTIER_REFS` deps.
pub const MAX_CUT_HEADS: usize = MAX_FRONTIER_REFS - 1;

/// The checkpoint digest endorsements are counted by: BLAKE3
/// `derive_key(GOV_CHECKPOINT)` over every field but `pages` and `round`
/// (page locators differ by writer while the content does not; the round
/// only numbers the vote).
#[must_use]
pub fn checkpoint_digest(c: &Checkpoint) -> [u8; 32] {
    let Checkpoint {
        height,
        round: _,
        prev,
        cut,
        frontier_hash,
        dir_root,
        dir_size,
        state_root,
        mek_generation,
        eligible_set_hash,
        pages: _,
    } = c;
    let mut h = blake3::Hasher::new_derive_key(domains::GOV_CHECKPOINT);
    h.update(&height.to_be_bytes());
    match prev {
        None => h.update(&[0]),
        Some(prev) => h.update(&[1]).update(&prev.op_hash.0),
    };
    let mut heads: Vec<OpHash> = cut.iter().map(|r| r.op_hash).collect();
    heads.sort_unstable();
    h.update(&u32::try_from(heads.len()).unwrap_or(u32::MAX).to_be_bytes());
    for head in heads {
        h.update(&head.0);
    }
    h.update(frontier_hash);
    h.update(dir_root);
    h.update(&dir_size.to_be_bytes());
    h.update(state_root);
    h.update(&mek_generation.to_be_bytes());
    h.update(eligible_set_hash);
    *h.finalize().as_bytes()
}

/// A cut's `frontierHash`: BLAKE3 `derive_key(GOV_CHECKPOINT_FRONTIER)`
/// over its head hashes, sorted.
#[must_use]
pub fn frontier_hash(heads: &[OpHash]) -> [u8; 32] {
    let mut sorted = heads.to_vec();
    sorted.sort_unstable();
    let mut h = blake3::Hasher::new_derive_key(domains::GOV_CHECKPOINT_FRONTIER);
    for head in sorted {
        h.update(&head.0);
    }
    *h.finalize().as_bytes()
}

/// The op-local rules of a `Checkpoint` op, refused on ingest: a cut of 1
/// to [`MAX_CUT_HEADS`] distinct heads, every one among the op's deps (so
/// whoever cites the op has the whole cut in its causal past), a
/// `frontierHash` over exactly those heads, and a previous checkpoint at
/// every height but 1. Any other op passes.
pub fn check_shape(op: &GovernanceOp) -> Result<(), OpRejection> {
    let GovernanceEntry::Checkpoint(c) = &op.body else {
        return Ok(());
    };
    let heads: Vec<OpHash> = c.cut.iter().map(|r| r.op_hash).collect();
    let mut distinct = heads.clone();
    distinct.sort_unstable();
    distinct.dedup();
    let cited = heads
        .iter()
        .all(|h| op.deps.iter().any(|d| d.op_hash == *h));
    let chained = (c.height == 1) == c.prev.is_none() && c.height >= 1;
    let sized = !heads.is_empty() && heads.len() <= MAX_CUT_HEADS && distinct.len() == heads.len();
    if cited && chained && sized && c.frontier_hash == frontier_hash(&heads) {
        Ok(())
    } else {
        Err(OpRejection::BadCheckpoint)
    }
}
