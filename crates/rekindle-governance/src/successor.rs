//! Successor communities and transfers in the merged state (ADR 0011
//! "Successor community", plan E3.2 S7s.3e, lead decisions (24)–(26)).
//!
//! Every applied `CommunitySuccessor` op is kept in
//! [`GovernanceState::successors`](crate::state::GovernanceState), covered
//! by `stateRoot`, so a reader resuming from a checkpoint judges an
//! acceptance whose offer is in the cut exactly as the full merge does
//! (`merge::gates::successor_gate`, `merge::resume_gates::successor_gate`).

use std::collections::BTreeSet;

use rekindle_secrets::sign::verify_signature;
use rekindle_types::domains;
use rekindle_types::governance::{CommunitySuccessor, CreatorLink, GenesisCert, SuccessorShape};
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

/// The bytes a creator's old pseudonym signs to link itself to `new` in
/// the successor at `new_anchor` of community `old_anchor` (lead decision
/// (24)): `GOV_SUCCESSOR_LINK ‖ len ‖ old anchor ‖ len ‖ new anchor ‖ new`,
/// each anchor prefixed by its length (u32, big-endian) so no two anchor
/// pairs share bytes.
#[must_use]
pub fn link_signing_bytes(old_anchor: &str, new_anchor: &str, new: &PseudonymKey) -> Vec<u8> {
    let label = domains::GOV_SUCCESSOR_LINK.as_bytes();
    let mut out = Vec::with_capacity(label.len() + 8 + old_anchor.len() + new_anchor.len() + 32);
    out.extend_from_slice(label);
    for anchor in [old_anchor, new_anchor] {
        let len = u32::try_from(anchor.len()).unwrap_or(u32::MAX);
        out.extend_from_slice(&len.to_be_bytes());
        out.extend_from_slice(anchor.as_bytes());
    }
    out.extend_from_slice(&new.0);
    out
}

/// Whether `link.sig` is `link.old`'s signature binding it to `link.new`
/// in the successor at `new_anchor` of community `old_anchor`.
#[must_use]
pub fn link_holds(old_anchor: &str, new_anchor: &str, link: &CreatorLink) -> bool {
    let Ok(sig) = <[u8; 64]>::try_from(link.sig.as_slice()) else {
        return false;
    };
    verify_signature(
        &link.old.0,
        &link_signing_bytes(old_anchor, new_anchor, &link.new),
        &sig,
    )
    .is_ok()
}

/// Why an applied successor op is not offered to the members.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuccessorRefusal {
    /// An offer or a withdrawal: nothing to follow yet.
    NotASuccessor,
    /// The new anchor's cert does not name this community as its
    /// `predecessor` (the far link is missing).
    NoPredecessorLink,
    /// A creator link's signature fails.
    BadLink,
    /// The links' new pseudonyms are not exactly the cert's creator set.
    CreatorSetMismatch,
    /// The recipient's link does not name the cert's `creatorPseudonym`.
    RecipientNotCreator,
}

/// The creator set to offer for successor `row` of community `community`,
/// judged against the verified cert of its new anchor (ADR 0011: "readers
/// treat it as a successor only when the new anchor's `GenesisCert.
/// predecessor` names this community"; lead decision (24)). The cert must
/// name `community` as its predecessor; for a transfer acceptance every
/// link must hold, the links' new pseudonyms must be the cert's creator
/// set, and the recipient's (the acceptance's author, the offer's
/// `transferTo` by `merge::gates::successor_gate`) must be its
/// `creatorPseudonym`.
///
/// # Errors
/// The [`SuccessorRefusal`] that keeps the successor from being offered.
pub fn offered_creators(
    community: &str,
    row: &SuccessorState,
    cert: &GenesisCert,
) -> Result<Vec<PseudonymKey>, SuccessorRefusal> {
    // Only a successor and an acceptance name a new anchor (`shape`).
    let shape = row.successor.shape();
    let Some(new_anchor) = row.successor.new_anchor.as_deref() else {
        return Err(SuccessorRefusal::NotASuccessor);
    };
    if cert.predecessor.as_deref() != Some(community) {
        return Err(SuccessorRefusal::NoPredecessorLink);
    }
    let creators: Vec<PseudonymKey> = std::iter::once(cert.creator_pseudonym.clone())
        .chain(cert.additional_creators.iter().cloned())
        .collect();
    if shape == Some(SuccessorShape::Successor) {
        return Ok(creators);
    }
    let links = &row.successor.links;
    if !links.iter().all(|l| link_holds(community, new_anchor, l)) {
        return Err(SuccessorRefusal::BadLink);
    }
    let linked: BTreeSet<[u8; 32]> = links.iter().map(|l| l.new.0).collect();
    let named: BTreeSet<[u8; 32]> = creators.iter().map(|c| c.0).collect();
    if linked.len() != links.len() || linked != named || named.len() != creators.len() {
        return Err(SuccessorRefusal::CreatorSetMismatch);
    }
    let recipient = links.iter().find(|l| l.old == row.author);
    if recipient.is_none_or(|l| l.new != cert.creator_pseudonym) {
        return Err(SuccessorRefusal::RecipientNotCreator);
    }
    Ok(creators)
}

#[cfg(test)]
#[path = "successor_tests.rs"]
mod tests;
