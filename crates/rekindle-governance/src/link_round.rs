//! A transfer's link round inside the old community (lead decision (46),
//! plan E3.2 S7s.4b). The offer's recipient writes a
//! `SuccessorLinkRequest` naming the offer's other creators by their
//! pseudonyms here; each answers with a `SuccessorLink` citing it. Both are
//! kept in [`GovernanceState::successor_links`](crate::state::GovernanceState),
//! covered by `stateRoot`, so a reader resuming from a checkpoint judges a
//! link whose request is in the cut as the full merge does, and the
//! acceptance's links are read from the merged op log.

use serde::{Deserialize, Serialize};

use rekindle_types::governance::{
    CommunitySuccessor, CreatorLink, GovernanceEntry, OpHash, OpRef, SuccessorLink,
    SuccessorLinkRequest, SuccessorShape,
};
use rekindle_types::id::PseudonymKey;

use crate::op::OpStamp;
use crate::state::GovernanceState;
use crate::successor::{accepts_offer, successor_set_hash};

/// A link-round op's body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkRoundBody {
    Request(SuccessorLinkRequest),
    Link(SuccessorLink),
}

/// One applied link-round op.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkRoundState {
    pub author: PseudonymKey,
    pub body: LinkRoundBody,
    pub stamp: OpStamp,
}

/// Whether `request`, by `writer`, fits `offer`: an offer to `writer`
/// whose creator set is exactly `request.asked` and `writer`.
#[must_use]
pub fn requests_offer(
    writer: &PseudonymKey,
    request: &SuccessorLinkRequest,
    offer: &CommunitySuccessor,
) -> bool {
    offer.shape() == Some(SuccessorShape::Offer)
        && offer.transfer_to.as_ref() == Some(writer)
        && !request.asked.contains(writer)
        && offer.creator_set_hash
            == Some(successor_set_hash(
                request.asked.iter().chain(std::iter::once(writer)),
            ))
}

/// The offer an op cites and must hold standing: an acceptance's
/// `accepts`, a link request's `offer`.
#[must_use]
pub fn cited_offer(body: &GovernanceEntry) -> Option<&OpRef> {
    match body {
        GovernanceEntry::CommunitySuccessor(acc) => acc.accepts.as_ref(),
        GovernanceEntry::SuccessorLinkRequest(r) => Some(&r.offer),
        _ => None,
    }
}

/// Whether `body`, by `writer`, fits the `offer` it cites.
#[must_use]
pub fn fits_offer(
    writer: &PseudonymKey,
    body: &GovernanceEntry,
    offer: &CommunitySuccessor,
) -> bool {
    match body {
        GovernanceEntry::CommunitySuccessor(acc) => accepts_offer(writer, acc, offer),
        GovernanceEntry::SuccessorLinkRequest(r) => requests_offer(writer, r, offer),
        _ => false,
    }
}

/// Whether `link`, by `writer`, answers `request`: `writer` is asked in
/// it and signs its own link.
#[must_use]
pub fn answers_request(
    writer: &PseudonymKey,
    link: &SuccessorLink,
    request: &SuccessorLinkRequest,
) -> bool {
    link.link.old == *writer && request.asked.contains(writer)
}

/// Whether acceptance `acc`, by `writer`, cites among its `deps` the
/// transfer's link round as kept in `state` (lead decision (46): the
/// transfer verifiable from the op log): an applied link request by
/// `writer` for the offer it accepts, naming its successor, and for every
/// other creator its links name, that creator's kept link answering that
/// request with exactly the link the acceptance carries. A sole creator's
/// acceptance carries only its own link and cites none; a body that is
/// not an acceptance is not judged here.
#[must_use]
pub fn acceptance_links_cited(
    writer: &PseudonymKey,
    body: &GovernanceEntry,
    deps: &[OpRef],
    state: &GovernanceState,
) -> bool {
    let GovernanceEntry::CommunitySuccessor(acc) = body else {
        return true;
    };
    let Some(offer) = &acc.accepts else {
        return true;
    };
    let others: Vec<&CreatorLink> = acc.links.iter().filter(|l| l.old != *writer).collect();
    if others.is_empty() {
        return true;
    }
    let row = |d: &OpRef| state.successor_links.get(&d.op_hash);
    let linked = |request: &OpHash, l: &CreatorLink| {
        deps.iter().any(|e| {
            row(e).is_some_and(|k| {
                k.author == l.old
                    && matches!(&k.body, LinkRoundBody::Link(link)
                        if link.request.op_hash == *request && link.link == *l)
            })
        })
    };
    deps.iter().any(|d| {
        row(d).is_some_and(|q| {
            q.author == *writer
                && matches!(&q.body, LinkRoundBody::Request(r)
                    if r.offer.op_hash == offer.op_hash
                        && acc.new_anchor.as_deref() == Some(r.new_anchor.as_str())
                        && others.iter().all(|l| r.asked.contains(&l.old) && linked(&d.op_hash, l)))
        })
    })
}

/// Keep an applied link-round op. A creator's first link to a request by
/// merge order is its answer; a later one is dropped (one link per
/// creator per request).
pub fn apply(
    author: &PseudonymKey,
    entry: &GovernanceEntry,
    stamp: OpStamp,
    state: &mut GovernanceState,
) {
    let body = match entry {
        GovernanceEntry::SuccessorLinkRequest(r) => LinkRoundBody::Request(r.clone()),
        GovernanceEntry::SuccessorLink(l) => {
            if links_to(state, &l.request.op_hash).any(|(_, a, _)| a == author) {
                return;
            }
            LinkRoundBody::Link(l.clone())
        }
        _ => return,
    };
    state.successor_links.insert(
        stamp.hash,
        LinkRoundState {
            author: author.clone(),
            body,
            stamp,
        },
    );
}

/// The applied links answering `request`: (link op, author, link).
pub fn links_to<'a>(
    state: &'a GovernanceState,
    request: &'a OpHash,
) -> impl Iterator<Item = (&'a OpHash, &'a PseudonymKey, &'a SuccessorLink)> + 'a {
    state
        .successor_links
        .iter()
        .filter_map(move |(hash, row)| match &row.body {
            LinkRoundBody::Link(l) if l.request.op_hash == *request => Some((hash, &row.author, l)),
            _ => None,
        })
}

/// The applied requests by `author` citing `offer`, by merge order.
pub fn requests_for<'a>(
    state: &'a GovernanceState,
    author: &'a PseudonymKey,
    offer: &'a OpHash,
) -> impl Iterator<Item = (&'a OpHash, &'a LinkRoundState, &'a SuccessorLinkRequest)> + 'a {
    state
        .successor_links
        .iter()
        .filter_map(move |(hash, row)| match &row.body {
            LinkRoundBody::Request(r) if row.author == *author && r.offer.op_hash == *offer => {
                Some((hash, row, r))
            }
            _ => None,
        })
}

#[cfg(test)]
#[path = "link_round_tests.rs"]
mod tests;
