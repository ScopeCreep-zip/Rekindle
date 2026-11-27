//! The endorsement chain a joiner checks before it uses a hint (plan E3.2
//! S7c.5): the height-1 hint endorsed by a strict majority of the cert's
//! creator set, and each height-h + 1 hint by a strict majority of the
//! eligible set at the height-h cut (the state its verified snapshot
//! holds), its endorsements naming a height-h endorsement as
//! `prevCheckpoint`. The joiner holds only the endorsements it fetched,
//! not their deps, so each is checked by signature and shape alone; its
//! hash is what the inviter's ref named.

use std::collections::BTreeSet;

use rekindle_secrets::sign::verify_signature;
use rekindle_types::governance::{Checkpoint, GenesisCert, GovernanceEntry, GovernanceOp, OpHash};
use rekindle_types::id::PseudonymKey;

use super::directory::{DirHash, DirectoryPage, DIR_PAGE_LEAVES};
use super::electorate::{eligible_set, is_strict_majority, tally};
use super::shape::{check_shape, checkpoint_digest};
use crate::op::{op_hash, op_signing_bytes};
use crate::state::GovernanceState;

/// A `Checkpoint` op whose signature verified, with its hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedEndorsement {
    pub hash: OpHash,
    pub author: PseudonymKey,
    pub body: Checkpoint,
}

impl SignedEndorsement {
    /// `op` (decoded from `unsigned` and its signature) of community
    /// `anchor`, when its device key's signature verifies (ADR 0022 rule
    /// 1: the device key signs, the author is named), its shape is a
    /// checkpoint's, and its body is a `Checkpoint`.
    #[must_use]
    pub fn verify(anchor: &str, op: &GovernanceOp, unsigned: &[u8]) -> Option<Self> {
        verify_signature(&op.device_key, &op_signing_bytes(anchor, unsigned), &op.sig).ok()?;
        check_shape(op).ok()?;
        let GovernanceEntry::Checkpoint(body) = &op.body else {
            return None;
        };
        Some(Self {
            hash: op_hash(unsigned, &op.sig),
            author: op.author.clone(),
            body: body.clone(),
        })
    }
}

/// One verified height of the chain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndorsedLink {
    pub height: u64,
    pub digest: [u8; 32],
    /// The body, as its endorsements carry it.
    pub body: Checkpoint,
    /// The endorsements' hashes: the next height's `prevCheckpoint` names
    /// one of them.
    pub ops: BTreeSet<OpHash>,
}

/// The link `endorsements` of community `anchor` make on top of `below` (the verified link one
/// height down and the state at its cut; `None` at height 1): every one
/// carries one digest at one height and round, its height is one above
/// `below`'s (1 with none), its `prevCheckpoint` is one of `below`'s ops
/// (none at height 1), and their distinct authors are a strict majority of
/// the eligible set at `below`'s cut (the cert's creator set at height 1),
/// counted by `tally`, which drops an author outside it or an
/// `eligibleSetHash` naming another set. `None` otherwise.
#[must_use]
pub fn endorsed_link(
    cert: &GenesisCert,
    anchor: &str,
    below: Option<(&EndorsedLink, &GovernanceState)>,
    endorsements: &[SignedEndorsement],
) -> Option<EndorsedLink> {
    let first = endorsements.first()?;
    let digest = checkpoint_digest(&first.body);
    let height = below.map_or(1, |(link, _)| link.height + 1);
    let fits = |e: &SignedEndorsement| {
        checkpoint_digest(&e.body) == digest
            && e.body.height == height
            && e.body.round == first.body.round
            && match (below, &e.body.prev) {
                (None, None) => true,
                (Some((link, _)), Some(prev)) => link.ops.contains(&prev.op_hash),
                _ => false,
            }
    };
    if !endorsements.iter().all(fits) {
        return None;
    }
    let electorate = eligible_set(cert, anchor, below.map(|(_, state)| state));
    let votes = tally(
        &electorate,
        endorsements.iter().map(|e| (&e.author, &e.body)),
    );
    let endorsers = votes.get(&digest)?;
    if !is_strict_majority(endorsers.len(), electorate.len()) {
        return None;
    }
    Some(EndorsedLink {
        height,
        digest,
        body: first.body.clone(),
        ops: endorsements.iter().map(|e| e.hash).collect(),
    })
}

/// Whether `pages`, in page order, are exactly `body`'s directory: one per
/// page key, each verifying against its `dirRoot` and `dirSize` for
/// `community_id` at its height, and together holding `dirSize` leaves.
#[must_use]
pub fn pages_match(community_id: &[u8; 32], body: &Checkpoint, pages: &[DirectoryPage]) -> bool {
    let root = DirHash(body.dir_root);
    let leaves: usize = pages.iter().map(|p| p.leaves.len()).sum();
    pages.len() == body.pages.len()
        && u64::try_from(leaves).is_ok_and(|n| n == body.dir_size)
        && (0u64..).zip(pages).all(|(i, p)| {
            u64::try_from(DIR_PAGE_LEAVES).is_ok_and(|per_page| p.first_index == i * per_page)
                && p.verify(community_id, body.height, body.dir_size, &root)
                    .is_ok()
        })
}

#[cfg(test)]
#[path = "chain_tests.rs"]
mod tests;
