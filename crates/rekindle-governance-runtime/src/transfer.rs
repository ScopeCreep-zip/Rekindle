//! Community transfer (ADR 0011 "Successor community", plan E3.2 S7s.4,
//! lead decisions (16), (24)–(26)). A creator offers the community to a
//! member (`transferTo`) over `creatorSetHash`, the hash of the new
//! creator set's old pseudonyms: the creators without the signer, with the
//! recipient. The signer's latest offer or withdrawal supersedes its
//! earlier ones (decision (26); `merge::gates::successor_gate` judges it
//! for every reader). The recipient accepts by running decision (16)'s
//! round with the offer's other creators, each signing a `CreatorLink`
//! from its old pseudonym to its pseudonym in the new anchor (decision
//! (24)); the round removes no one (decision (25)). It then creates the
//! successor (its cert names this community as `predecessor`, the links'
//! new pseudonyms as its creator set) and publishes the acceptance here,
//! citing the offer among its deps.

use std::collections::BTreeSet;

use rekindle_governance::state::GovernanceState;
use rekindle_governance::successor::{link_holds, successor_set_hash, SuccessorState};
use rekindle_types::governance::{
    CommunitySuccessor, CreatorLink, GovernanceEntry, OpHash, OpRef, SuccessorReason,
    SuccessorShape,
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
/// is cited among its deps, so every reader's `successor_gate` finds it.
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
    invites_record: &str,
) -> Result<(), GovernanceRuntimeError> {
    let me = own_pseudonym(deps, community_id)?;
    check_links(community_id, new_anchor, round, &me, &links)?;
    let offer = crate::op_log::held_op_ref(deps, community_id, round.offer).await?;
    let entry = acceptance_entry(offer, round.set_hash, new_anchor, links, invites_record);
    crate::apply::write_entry_citing(deps, community_id, entry, vec![round.offer])
        .await
        .map(drop)
}

#[cfg(test)]
#[path = "transfer_tests.rs"]
mod tests;
