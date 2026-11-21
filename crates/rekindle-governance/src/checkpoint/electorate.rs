//! Who endorses a checkpoint and when a digest is endorsed (plan E3.2
//! S7c.3, ADR 0018): the electorate is fixed by the previous checkpoint,
//! endorsements count once per member, and a signature made for an old
//! set never counts toward a new one.

use std::collections::{BTreeMap, BTreeSet};

use rekindle_types::domains;
use rekindle_types::governance::{Checkpoint, GenesisCert};
use rekindle_types::id::PseudonymKey;

use super::shape::checkpoint_digest;
use crate::state::GovernanceState;

/// The electorate of the checkpoint after `prev_cut_state`: the admitted,
/// non-removed members of the state at the previous checkpoint's cut,
/// sorted; at height 1 (no previous checkpoint) the cert's creator set.
#[must_use]
pub fn eligible_set(
    cert: &GenesisCert,
    prev_cut_state: Option<&GovernanceState>,
) -> Vec<PseudonymKey> {
    let mut set: Vec<PseudonymKey> = match prev_cut_state {
        None => cert.creator_set().into_iter().collect(),
        Some(state) => (state.members.keys())
            .filter(|m| !state.is_departed(m))
            .cloned()
            .collect(),
    };
    set.sort_by_key(|m| m.0);
    set
}

/// `eligibleSetHash`: BLAKE3 `derive_key(GOV_CHECKPOINT_ELIGIBLE)` over
/// the electorate's pseudonyms in byte order.
#[must_use]
pub fn eligible_set_hash(set: &[PseudonymKey]) -> [u8; 32] {
    let sorted: BTreeSet<[u8; 32]> = set.iter().map(|m| m.0).collect();
    let mut h = blake3::Hasher::new_derive_key(domains::GOV_CHECKPOINT_ELIGIBLE);
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

/// The endorsers (pseudonym bytes) of each checkpoint digest among
/// `electorate`: one per member however many ops it signed for that
/// digest, and only from ops whose `eligibleSetHash` names this
/// electorate.
#[must_use]
pub fn tally<'a>(
    electorate: &[PseudonymKey],
    endorsements: impl IntoIterator<Item = (&'a PseudonymKey, &'a Checkpoint)>,
) -> BTreeMap<[u8; 32], BTreeSet<[u8; 32]>> {
    let set_hash = eligible_set_hash(electorate);
    let mut out: BTreeMap<[u8; 32], BTreeSet<[u8; 32]>> = BTreeMap::new();
    for (author, c) in endorsements {
        if c.eligible_set_hash == set_hash && electorate.contains(author) {
            (out.entry(checkpoint_digest(c)).or_default()).insert(author.0);
        }
    }
    out
}

/// Whether `endorsers` are a strict majority of `electorate` (ADR 0018).
#[must_use]
pub fn is_strict_majority(endorsers: usize, electorate: usize) -> bool {
    endorsers.saturating_mul(2) > electorate
}
