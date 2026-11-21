//! Successor communities and transfers in the merged state (ADR 0011
//! "Successor community", plan E3.2 S7s.3e, lead decisions (24)–(26)).
//!
//! Every applied `CommunitySuccessor` op is kept in
//! [`GovernanceState::successors`](crate::state::GovernanceState), covered
//! by `stateRoot`, so a reader resuming from a checkpoint judges an
//! acceptance whose offer is in the cut exactly as the full merge does
//! (`merge::gates::successor_gate`, `merge::resume_gates::successor_gate`).

use std::collections::BTreeSet;

use rekindle_types::domains;
use rekindle_types::governance::{CommunitySuccessor, SuccessorShape};
use rekindle_types::id::PseudonymKey;

use crate::op::OpStamp;

/// One applied `CommunitySuccessor` op.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuccessorState {
    pub author: PseudonymKey,
    pub successor: CommunitySuccessor,
    pub stamp: OpStamp,
}

/// `creatorSetHash`: BLAKE3 `derive_key(GOV_SUCCESSOR_SET)` over the old
/// pseudonyms of a transfer's new creator set, in byte order (decision
/// (24)), each counted once.
#[must_use]
pub fn successor_set_hash<'a>(set: impl IntoIterator<Item = &'a PseudonymKey>) -> [u8; 32] {
    let sorted: BTreeSet<[u8; 32]> = set.into_iter().map(|m| m.0).collect();
    let mut h = blake3::Hasher::new_derive_key(domains::GOV_SUCCESSOR_SET);
    h.update(
        &u32::try_from(sorted.len())
            .unwrap_or(u32::MAX)
            .to_be_bytes(),
    );
    for m in sorted {
        h.update(&m);
    }
    *h.finalize().as_bytes()
}

/// Whether acceptance `acc`, written by `writer`, accepts `offer` (an
/// applied offer among its deps, by `offer_author`, not superseded in its
/// causal past): the writer is the offer's `transferTo`, the acceptance
/// repeats the offer's `creatorSetHash`, and its links' old pseudonyms are
/// exactly the set that hash names, `transferTo` among them (decisions
/// (24), (26)). The new anchor's cert and each link's signature are the
/// successor reader's to check (S7s.3e part 2).
#[must_use]
pub fn accepts_offer(
    writer: &PseudonymKey,
    acc: &CommunitySuccessor,
    offer: &CommunitySuccessor,
) -> bool {
    if offer.shape() != Some(SuccessorShape::Offer) {
        return false;
    }
    let Some(to) = &offer.transfer_to else {
        return false;
    };
    writer == to
        && acc.creator_set_hash == offer.creator_set_hash
        && acc.links.iter().any(|l| l.old == *to)
        && offer.creator_set_hash == Some(successor_set_hash(acc.links.iter().map(|l| &l.old)))
}
