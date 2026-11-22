//! Community transfer (ADR 0011 "Successor community", plan E3.2 S7s.4,
//! lead decisions (16), (24)–(26)). A creator offers the community to a
//! member (`transferTo`) over `creatorSetHash`, the hash of the new
//! creator set's old pseudonyms: the creators without the signer, with the
//! recipient. The signer's latest offer or withdrawal supersedes its
//! earlier ones (decision (26); `merge::gates::successor_gate` judges it
//! for every reader). The recipient accepts by running the link round
//! inside this community (lead decision (46)): its `SuccessorLinkRequest`
//! asks the offer's other creators, by their pseudonyms here, and each
//! answers with a `SuccessorLink` op signing a `CreatorLink` from its old
//! pseudonym to its pseudonym in the new anchor (decision (24)), with its
//! co-creator acceptance. The round removes no one (decision (25)). The
//! recipient then creates the successor (its cert names this community as
//! `predecessor`, the links' new pseudonyms as its creator set) and
//! publishes the acceptance here, citing the offer and the link ops.

use std::collections::BTreeSet;

use rekindle_governance::link_round::{links_to, requests_for, LinkRoundBody};
use rekindle_governance::state::GovernanceState;
use rekindle_governance::successor::{link_holds, successor_set_hash, SuccessorState};
use rekindle_types::governance::{
    CommunitySuccessor, CreatorLink, GovernanceEntry, OpHash, OpRef, SuccessorLink,
    SuccessorLinkRequest, SuccessorReason, SuccessorShape,
};
use rekindle_types::id::PseudonymKey;

use crate::deps::GovernanceRuntimeDeps;
use crate::error::GovernanceRuntimeError;

/// The new creator set of `signer`'s transfer to `to`: the creators
/// without the signer, with the recipient (old pseudonyms).
#[must_use]
pub fn transfer_set(
    state: &GovernanceState,
    signer: &PseudonymKey,
    to: &PseudonymKey,
) -> BTreeSet<PseudonymKey> {
    (state.creators.iter())
        .filter(|c| *c != signer)
        .chain(std::iter::once(to))
        .cloned()
        .collect()
}

/// `signer`'s offer of the community to `to`.
///
/// # Errors
/// `signer` is not a creator (only a creator offers, ADR 0011), `to` is the
/// signer, or `to` is not a counted member.
pub fn offer_entry(
    state: &GovernanceState,
    signer: &PseudonymKey,
    to: &PseudonymKey,
) -> Result<GovernanceEntry, GovernanceRuntimeError> {
    if !rekindle_governance::permissions::holds_creator_seat(signer, state) {
        return Err(GovernanceRuntimeError::PermissionDenied);
    }
    if to == signer || !state.is_counted_member(to) {
        return Err(GovernanceRuntimeError::OpRefused(
            "a transfer goes to another member of the community".into(),
        ));
    }
    let set = transfer_set(state, signer, to);
    Ok(GovernanceEntry::CommunitySuccessor(CommunitySuccessor {
        new_anchor: None,
        reason: SuccessorReason::Transfer,
        transfer_to: Some(to.clone()),
        creator_set_hash: Some(successor_set_hash(set.iter())),
        accepts: None,
        links: Vec::new(),
        invites_record: None,
    }))
}

/// A withdrawal: it supersedes the signer's earlier offers (decision (26)).
#[must_use]
pub fn withdrawal_entry() -> GovernanceEntry {
    GovernanceEntry::CommunitySuccessor(CommunitySuccessor {
        new_anchor: None,
        reason: SuccessorReason::Transfer,
        transfer_to: None,
        creator_set_hash: None,
        accepts: None,
        links: Vec::new(),
        invites_record: None,
    })
}

/// Each signer's standing offer: its latest offer or withdrawal by merge
/// order, kept when it is an offer (decision (26)).
#[must_use]
pub fn standing_offers(state: &GovernanceState) -> Vec<(&OpHash, &SuccessorState)> {
    let mut latest: std::collections::BTreeMap<&PseudonymKey, (&OpHash, &SuccessorState)> =
        std::collections::BTreeMap::new();
    for (hash, row) in &state.successors {
        if !row.successor.is_offer_or_withdrawal() {
            continue;
        }
        let newer = latest
            .get(&row.author)
            .is_none_or(|(_, held)| held.stamp < row.stamp);
        if newer {
            latest.insert(&row.author, (hash, row));
        }
    }
    (latest.into_values())
        .filter(|(_, row)| row.successor.shape() == Some(SuccessorShape::Offer))
        .collect()
}

/// What the recipient's round needs: the standing offer to `me` (the
/// latest by merge order when several creators offered), its signer, its
/// set hash and the old pseudonyms whose links it collects (the set
/// without `me`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferRound {
    pub offer: OpHash,
    pub offerer: PseudonymKey,
    pub set_hash: [u8; 32],
    pub others: BTreeSet<PseudonymKey>,
}

/// The round for accepting the standing offer to `me`.
///
/// # Errors
/// No standing offer names us, or the creator set has changed since it
/// was made (its hash no longer matches: the offerer must offer again).
pub fn transfer_round(
    state: &GovernanceState,
    me: &PseudonymKey,
) -> Result<TransferRound, GovernanceRuntimeError> {
    let (hash, row) = standing_offers(state)
        .into_iter()
        .filter(|(_, row)| row.successor.transfer_to.as_ref() == Some(me))
        .max_by_key(|(_, row)| row.stamp)
        .ok_or_else(|| GovernanceRuntimeError::OpRefused("no standing offer names us".into()))?;
    let set = transfer_set(state, &row.author, me);
    let set_hash = successor_set_hash(set.iter());
    if row.successor.creator_set_hash != Some(set_hash) {
        return Err(GovernanceRuntimeError::OpRefused(
            "the creator set changed since the offer: it must be offered again".into(),
        ));
    }
    Ok(TransferRound {
        offer: *hash,
        offerer: row.author.clone(),
        set_hash,
        others: set.into_iter().filter(|c| c != me).collect(),
    })
}

/// Check the collected `links` before the successor is created: each
/// holds for `old_anchor` → `new_anchor`, and their old pseudonyms are
/// exactly the round's set (`round.others` and `me`), each once.
///
/// # Errors
/// A link fails, or a creator of the set is not linked (or one outside it
/// is).
pub fn check_links(
    old_anchor: &str,
    new_anchor: &str,
    round: &TransferRound,
    me: &PseudonymKey,
    links: &[CreatorLink],
) -> Result<(), GovernanceRuntimeError> {
    if let Some(bad) = links
        .iter()
        .find(|l| !link_holds(old_anchor, new_anchor, l))
    {
        return Err(GovernanceRuntimeError::Crypto(format!(
            "creator link of {} does not hold",
            bad.old.to_hex()
        )));
    }
    let linked: BTreeSet<&PseudonymKey> = links.iter().map(|l| &l.old).collect();
    let wanted: BTreeSet<&PseudonymKey> = round.others.iter().chain(std::iter::once(me)).collect();
    if linked.len() != links.len() || linked != wanted {
        return Err(GovernanceRuntimeError::OpRefused(
            "the links do not cover exactly the offer's creator set".into(),
        ));
    }
    Ok(())
}

/// The acceptance op: it names `new_anchor`, accepts `offer`, repeats its
/// set hash, carries the links and the continuing members' sealed invites
/// (lead decision (34)).
#[must_use]
pub fn acceptance_entry(
    offer: OpRef,
    set_hash: [u8; 32],
    new_anchor: &str,
    links: Vec<CreatorLink>,
    invites_record: &str,
) -> GovernanceEntry {
    GovernanceEntry::CommunitySuccessor(CommunitySuccessor {
        new_anchor: Some(new_anchor.to_owned()),
        reason: SuccessorReason::Transfer,
        transfer_to: None,
        creator_set_hash: Some(set_hash),
        accepts: Some(offer),
        links,
        invites_record: Some(invites_record.to_owned()),
    })
}

/// Our pseudonym in `community_id`, from the membership.
fn own_pseudonym<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
) -> Result<PseudonymKey, GovernanceRuntimeError> {
    deps.community_membership(community_id)
        .and_then(|m| m.my_pseudonym_hex)
        .map(|hex| PseudonymKey::from_hex_lossy(&hex))
        .ok_or_else(|| GovernanceRuntimeError::PseudonymKeyMissing(community_id.to_owned()))
}

fn state_of<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
) -> Result<GovernanceState, GovernanceRuntimeError> {
    deps.governance_state(community_id)
        .ok_or_else(|| GovernanceRuntimeError::GovernanceStateMissing(community_id.to_owned()))
}

/// Offer `community_id` to `to` (plan E3.2 S7s.4, intent
/// `OfferCommunityTransfer`).
///
/// # Errors
/// As [`offer_entry`], or the op was refused or not written.
pub async fn offer<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
    to: &PseudonymKey,
) -> Result<(), GovernanceRuntimeError> {
    let state = state_of(deps, community_id)?;
    let entry = offer_entry(&state, &own_pseudonym(deps, community_id)?, to)?;
    crate::apply::write_entry(deps, community_id, entry)
        .await
        .map(drop)
}

/// Withdraw our standing offer of `community_id` (decision (26)).
///
/// # Errors
/// We have no standing offer, or the op was refused or not written.
pub async fn withdraw<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
) -> Result<(), GovernanceRuntimeError> {
    let state = state_of(deps, community_id)?;
    let me = own_pseudonym(deps, community_id)?;
    if !standing_offers(&state).iter().any(|(_, r)| r.author == me) {
        return Err(GovernanceRuntimeError::OpRefused(
            "we have no standing transfer offer to withdraw".into(),
        ));
    }
    crate::apply::write_entry(deps, community_id, withdrawal_entry())
        .await
        .map(drop)
}

/// The round for our acceptance of the standing offer of `community_id`.
///
/// # Errors
/// As [`transfer_round`].
pub fn round_for<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
) -> Result<(TransferRound, PseudonymKey), GovernanceRuntimeError> {
    let state = state_of(deps, community_id)?;
    let me = own_pseudonym(deps, community_id)?;
    Ok((transfer_round(&state, &me)?, me))
}

/// Publish our acceptance of `round`'s offer in `community_id`: the offer
/// and the link ops (`link_ops`) are cited among its deps, so every
/// reader's `successor_gate` finds the offer and the whole transfer is
/// verifiable from the op log (lead decision (46)).
///
/// # Errors
/// The links fail ([`check_links`]), the offer is not held, or the op was
/// refused or not written.
pub async fn publish_acceptance<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
    round: &TransferRound,
    new_anchor: &str,
    links: Vec<CreatorLink>,
    link_ops: &[OpHash],
    invites_record: &str,
) -> Result<(), GovernanceRuntimeError> {
    let me = own_pseudonym(deps, community_id)?;
    check_links(community_id, new_anchor, round, &me, &links)?;
    let offer = crate::op_log::held_op_ref(deps, community_id, round.offer).await?;
    let entry = acceptance_entry(offer, round.set_hash, new_anchor, links, invites_record);
    let cites = std::iter::once(round.offer).chain(link_ops.iter().copied());
    crate::apply::write_entry_citing(deps, community_id, entry, cites.collect())
        .await
        .map(drop)
}

/// The link request for `round` (lead decision (46)).
#[must_use]
pub fn request_entry(offer: OpRef, new_anchor: &str, round: &TransferRound) -> GovernanceEntry {
    GovernanceEntry::SuccessorLinkRequest(SuccessorLinkRequest {
        offer,
        new_anchor: new_anchor.to_owned(),
        asked: round.others.iter().cloned().collect(),
    })
}

/// A co-creator's link answering `request` (lead decision (46)).
#[must_use]
pub fn link_entry(request: OpRef, link: CreatorLink, creator_sig: [u8; 64]) -> GovernanceEntry {
    GovernanceEntry::SuccessorLink(SuccessorLink {
        request,
        link,
        creator_sig: creator_sig.to_vec(),
    })
}

/// What our request for `round` naming `new_anchor` has collected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollectedLinks {
    pub request: OpHash,
    /// Each asked creator's link op and link that holds for the anchors.
    pub links: Vec<(OpHash, SuccessorLink)>,
    /// The asked creators without a holding link yet.
    pub missing: Vec<PseudonymKey>,
}

/// The links our latest request (by merge order) for `round` naming
/// `new_anchor` has collected from the merged op log, or `None` when no
/// such request is applied. A link that does not hold for the anchors
/// counts as missing.
#[must_use]
pub fn collected_links(
    state: &GovernanceState,
    old_anchor: &str,
    round: &TransferRound,
    me: &PseudonymKey,
    new_anchor: &str,
) -> Option<CollectedLinks> {
    let (request, _, _) = requests_for(state, me, &round.offer)
        .filter(|(_, _, r)| r.new_anchor == new_anchor)
        .max_by_key(|(_, row, _)| row.stamp)?;
    let links: Vec<(OpHash, SuccessorLink)> = links_to(state, request)
        .filter(|(_, author, l)| {
            round.others.contains(*author) && link_holds(old_anchor, new_anchor, &l.link)
        })
        .map(|(hash, _, l)| (*hash, l.clone()))
        .collect();
    let missing = (round.others.iter())
        .filter(|c| !links.iter().any(|(_, l)| l.link.old == **c))
        .cloned()
        .collect();
    Some(CollectedLinks {
        request: *request,
        links,
        missing,
    })
}

/// The offer's other creators who have not linked into the successor of
/// our latest request for the standing offer to `me` (all of them before
/// any request): the accept review's list (decision (25)).
///
/// # Errors
/// As [`transfer_round`].
pub fn links_missing(
    state: &GovernanceState,
    old_anchor: &str,
    me: &PseudonymKey,
) -> Result<Vec<PseudonymKey>, GovernanceRuntimeError> {
    let round = transfer_round(state, me)?;
    let latest = requests_for(state, me, &round.offer).max_by_key(|(_, row, _)| row.stamp);
    Ok(match latest {
        Some((_, _, r)) => collected_links(state, old_anchor, &round, me, &r.new_anchor)
            .map_or_else(|| round.others.iter().cloned().collect(), |c| c.missing),
        None => round.others.iter().cloned().collect(),
    })
}

/// The link requests asking `me` that it has not answered, each the
/// latest by its author for an offer still standing (decision (26)).
#[must_use]
pub fn requests_asking(
    state: &GovernanceState,
    me: &PseudonymKey,
) -> Vec<(OpHash, SuccessorLinkRequest)> {
    let standing: BTreeSet<OpHash> = standing_offers(state)
        .into_iter()
        .map(|(h, _)| *h)
        .collect();
    let mut latest: std::collections::BTreeMap<
        &PseudonymKey,
        (&OpHash, &rekindle_governance::link_round::LinkRoundState),
    > = std::collections::BTreeMap::new();
    for (hash, row) in &state.successor_links {
        let LinkRoundBody::Request(r) = &row.body else {
            continue;
        };
        if !r.asked.contains(me) || !standing.contains(&r.offer.op_hash) {
            continue;
        }
        if latest
            .get(&row.author)
            .is_none_or(|(_, held)| held.stamp < row.stamp)
        {
            latest.insert(&row.author, (hash, row));
        }
    }
    (latest.into_values())
        .filter(|(hash, _)| !links_to(state, hash).any(|(_, author, _)| author == me))
        .filter_map(|(hash, row)| match &row.body {
            LinkRoundBody::Request(r) => Some((*hash, r.clone())),
            LinkRoundBody::Link(_) => None,
        })
        .collect()
}

/// Ask the offer's other creators for their links into `new_anchor`
/// (lead decision (46)); the request cites the offer.
///
/// # Errors
/// The offer is not held, or the op was refused or not written.
pub async fn request_links<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
    round: &TransferRound,
    new_anchor: &str,
) -> Result<(), GovernanceRuntimeError> {
    let offer = crate::op_log::held_op_ref(deps, community_id, round.offer).await?;
    let entry = request_entry(offer, new_anchor, round);
    crate::apply::write_entry_citing(deps, community_id, entry, vec![round.offer])
        .await
        .map(drop)
}

/// Answer the request `request` asking us with our link and co-creator
/// acceptance (lead decision (46)); the link cites the request.
///
/// # Errors
/// No unanswered request with that hash asks us, the request is not held,
/// or the op was refused or not written.
pub async fn answer_link<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
    request: OpHash,
    link: CreatorLink,
    creator_sig: [u8; 64],
) -> Result<(), GovernanceRuntimeError> {
    let state = state_of(deps, community_id)?;
    let me = own_pseudonym(deps, community_id)?;
    if !requests_asking(&state, &me)
        .iter()
        .any(|(h, _)| *h == request)
    {
        return Err(GovernanceRuntimeError::OpRefused(
            "no unanswered link request with that hash asks us".into(),
        ));
    }
    let cited = crate::op_log::held_op_ref(deps, community_id, request).await?;
    let entry = link_entry(cited, link, creator_sig);
    crate::apply::write_entry_citing(deps, community_id, entry, vec![request])
        .await
        .map(drop)
}

#[cfg(test)]
#[path = "transfer_tests.rs"]
mod tests;
