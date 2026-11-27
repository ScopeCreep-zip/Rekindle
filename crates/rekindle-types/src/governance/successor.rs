//! The `CommunitySuccessor` op body (ADR 0011 "Successor community", plan
//! E3.2 S7s.3e, lead decisions (24)–(26)): a creator names the community
//! that succeeds this one, or offers it to a member (a transfer), and that
//! member accepts by naming the new anchor it created. ADR 0022 rule 8's
//! pseudonym-signed form (`cut`, `forkEvidence`, `pseudonymSig`) appends to
//! the same struct later (S7d-keys K8).

use serde::{Deserialize, Serialize};

use crate::id::PseudonymKey;

use super::op::{OpRef, MAX_ADDITIONAL_CREATORS};

/// Why a successor exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SuccessorReason {
    /// A creator moves the community to a new anchor it created.
    Recreate,
    /// A creator hands the community to a member: the offer, its
    /// withdrawal and the member's acceptance.
    Transfer,
}

/// One creator's old pseudonym bound to its pseudonym in the new community
/// (decision (24): pseudonyms are per community). `sig` is by `old` under
/// `rekindle-gov-successor-link-v1` over old anchor ‖ new anchor ‖ `new`,
/// checked by the successor reader (S7s.3e part 2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreatorLink {
    pub old: PseudonymKey,
    pub new: PseudonymKey,
    /// 64-byte Ed25519 signature; any other length fails verification.
    pub sig: Vec<u8>,
}

/// A `CommunitySuccessor` op's body. Which fields are set decides its
/// [`SuccessorShape`]; any other combination is malformed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommunitySuccessor {
    /// The new community's anchor key; unset in an offer or withdrawal.
    pub new_anchor: Option<String>,
    pub reason: SuccessorReason,
    /// The member a transfer offer names.
    pub transfer_to: Option<PseudonymKey>,
    /// BLAKE3 `derive_key("rekindle-gov-successor-set-v1")` over the old
    /// pseudonyms of the new creator set, sorted (decision (24)); carried
    /// by an offer and repeated by its acceptance.
    pub creator_set_hash: Option<[u8; 32]>,
    /// The offer an acceptance accepts; it must be among the op's deps.
    pub accepts: Option<OpRef>,
    /// An acceptance's creator links, one per member of the new set.
    pub links: Vec<CreatorLink>,
    /// A successor's sealed invites record (lead decision (34)): the key of
    /// the first DFLT(1) page holding one invite to the new community per
    /// continuing member, each HPKE-sealed to that member's card key, so no
    /// invite is readable in the op log itself. Set in a successor and a
    /// transfer acceptance only.
    pub invites_record: Option<String>,
}

/// The four well-formed shapes of a [`CommunitySuccessor`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuccessorShape {
    /// `newAnchor` and `invitesRecord`, reason `Recreate`: a creator's own
    /// successor.
    Successor,
    /// `transferTo` and `creatorSetHash`, reason `Transfer`.
    Offer,
    /// Nothing set, reason `Transfer`: the signer's earlier offers end.
    Withdrawal,
    /// `newAnchor`, `creatorSetHash`, `accepts` and 1–9 links with
    /// distinct old pseudonyms, reason `Transfer`.
    Acceptance,
}

impl CommunitySuccessor {
    /// The body's shape, or `None` when it is malformed.
    #[must_use]
    pub fn shape(&self) -> Option<SuccessorShape> {
        let anchor = self.new_anchor.as_ref().is_some_and(|a| !a.is_empty());
        if self.new_anchor.is_some() && !anchor {
            return None;
        }
        let set = self.creator_set_hash.is_some();
        let fields = (
            anchor,
            self.transfer_to.is_some(),
            set,
            self.accepts.is_some(),
        );
        let shape = match (self.reason, fields) {
            (SuccessorReason::Recreate, (true, false, false, false)) => SuccessorShape::Successor,
            (SuccessorReason::Transfer, (false, true, true, false)) => SuccessorShape::Offer,
            (SuccessorReason::Transfer, (false, false, false, false)) => SuccessorShape::Withdrawal,
            (SuccessorReason::Transfer, (true, false, true, true)) => SuccessorShape::Acceptance,
            _ => return None,
        };
        let links_ok = if shape == SuccessorShape::Acceptance {
            let mut old: Vec<_> = self.links.iter().map(|l| l.old.0).collect();
            old.sort_unstable();
            old.dedup();
            (1..=MAX_ADDITIONAL_CREATORS + 1).contains(&self.links.len())
                && old.len() == self.links.len()
        } else {
            self.links.is_empty()
        };
        // A successor and a transfer acceptance both name a new community
        // the members follow, so both carry its sealed invites (lead
        // decision (34)); an offer and a withdrawal carry none.
        let names_new = matches!(
            shape,
            SuccessorShape::Successor | SuccessorShape::Acceptance
        );
        let invites_ok = match &self.invites_record {
            Some(record) => names_new && !record.is_empty(),
            None => !names_new,
        };
        (links_ok && invites_ok).then_some(shape)
    }

    /// Whether the body is an offer or a withdrawal: the ops one signer's
    /// latest supersedes (decision (26)).
    #[must_use]
    pub fn is_offer_or_withdrawal(&self) -> bool {
        matches!(
            self.shape(),
            Some(SuccessorShape::Offer | SuccessorShape::Withdrawal)
        )
    }
}

/// A transfer recipient's request, inside the old community, that the
/// offer's other creators link themselves into the successor it reserved
/// (lead decision (46)): addressed by old pseudonym, so co-creators need
/// not be the recipient's friends (decision (25)).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SuccessorLinkRequest {
    /// The offer being accepted; it must be among the op's deps.
    pub offer: OpRef,
    /// The reserved successor's anchor key.
    pub new_anchor: String,
    /// The old pseudonyms asked: the offer's creator set without the
    /// recipient.
    pub asked: Vec<PseudonymKey>,
}

impl SuccessorLinkRequest {
    /// An anchor, and 1–9 distinct pseudonyms asked.
    #[must_use]
    pub fn well_formed(&self) -> bool {
        let mut asked: Vec<_> = self.asked.iter().map(|k| k.0).collect();
        asked.sort_unstable();
        asked.dedup();
        !self.new_anchor.is_empty()
            && (1..=MAX_ADDITIONAL_CREATORS).contains(&self.asked.len())
            && asked.len() == self.asked.len()
    }
}

/// A co-creator's answer to a [`SuccessorLinkRequest`] naming it (lead
/// decision (46)): its creator link (`link.old` is the op's author) and
/// its co-creator acceptance of the new anchor for the successor's cert.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SuccessorLink {
    /// The request answered; it must be among the op's deps.
    pub request: OpRef,
    pub link: CreatorLink,
    /// 64-byte Ed25519 signature by `link.new` over
    /// `creator_accept_signing_bytes(new anchor, link.new)`.
    pub creator_sig: Vec<u8>,
}

impl SuccessorLink {
    /// Both signatures are 64 bytes.
    #[must_use]
    pub fn well_formed(&self) -> bool {
        self.link.sig.len() == 64 && self.creator_sig.len() == 64
    }
}

#[cfg(test)]
#[path = "successor_tests.rs"]
mod tests;
