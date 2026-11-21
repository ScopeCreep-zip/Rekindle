//! The successor reader (ADR 0011 "Successor community", plan E3.2 S7s.3e
//! part 2, lead decisions (24) and (31)): an applied `CommunitySuccessor`
//! naming a new anchor is offered to the members only once the far side
//! links back. The reader fetches the new anchor's `GenesisCert` pinned to
//! its owner ([`crate::genesis::read_pinned`]) and judges it
//! ([`offered_creators`]: `predecessor` names this community and, for a
//! transfer acceptance, the creator links hold and are the cert's creator
//! set). It runs after each merge of the rebuild (`op_log::rebuild_ops`)
//! for every applied successor not yet judged ([`SuccessorJudgements`]).
//!
//! Decision (31): a far cert that cannot be read yet (the record or its
//! subkey unreadable, or not there yet) leaves the offer pending, re-read on
//! the next rebuild, so a transient read failure never hides a successor
//! for good. A cert that reads and fails a rule (it does not decode, is not
//! signed by its anchor's owner, its creator set or a signature fails, or
//! it does not link back) is refused with its reason and not read again.

use std::collections::BTreeSet;
use std::future::Future;

use rekindle_governance::state::GovernanceState;
use rekindle_governance::successor::{offered_creators, SuccessorState};
use rekindle_types::governance::{GenesisCert, OpHash};

use crate::deps::GovernanceRuntimeDeps;
use crate::error::GovernanceRuntimeError;
use crate::event::GovernanceRuntimeEvent;

/// One community's successors already offered or refused, by op hash.
/// Owned per community by the Host
/// (`GovernanceRuntimeDeps::successor_judgements`) for the session: after
/// a restart each applied successor is judged again, so an offer the member
/// has not acted on is shown again.
#[derive(Debug, Default)]
pub struct SuccessorJudgements {
    judged: parking_lot::Mutex<BTreeSet<OpHash>>,
}

impl SuccessorJudgements {
    fn is_judged(&self, hash: &OpHash) -> bool {
        self.judged.lock().contains(hash)
    }

    fn mark(&self, hash: OpHash) {
        self.judged.lock().insert(hash);
    }
}

/// The applied successors and acceptances (the rows naming a new anchor)
/// not yet judged. Two valid successors are both kept.
#[must_use]
pub fn pending<'a>(
    state: &'a GovernanceState,
    judgements: &SuccessorJudgements,
) -> Vec<(OpHash, &'a SuccessorState)> {
    (state.successors.iter())
        .filter(|(hash, row)| row.successor.new_anchor.is_some() && !judgements.is_judged(hash))
        .map(|(hash, row)| (*hash, row))
        .collect()
}

/// Whether `error` from reading a far cert means it could not be read
/// (pending, re-read on the next rebuild) rather than read and broken.
fn unreadable(error: &GovernanceRuntimeError) -> bool {
    !matches!(
        error,
        GovernanceRuntimeError::Encoding(_) | GovernanceRuntimeError::Crypto(_)
    )
}

/// Judge every pending successor with `read` (the far cert by new anchor):
/// the offers to publish. An unreadable cert stays pending; a refused one
/// is marked judged with a WARN, an offered one marked judged.
pub async fn judge<F, Fut>(
    community_id: &str,
    state: &GovernanceState,
    judgements: &SuccessorJudgements,
    mut read: F,
) -> Vec<GovernanceRuntimeEvent>
where
    F: FnMut(String) -> Fut,
    Fut: Future<Output = Result<GenesisCert, GovernanceRuntimeError>>,
{
    let mut offers = Vec::new();
    for (hash, row) in pending(state, judgements) {
        let Some(new_anchor) = row.successor.new_anchor.clone() else {
            continue;
        };
        let cert = match read(new_anchor.clone()).await {
            Ok(cert) => cert,
            Err(error) if unreadable(&error) => {
                tracing::info!(
                    community = community_id,
                    new_anchor,
                    ?error,
                    "a successor's new anchor cannot be read yet; re-read on the next rebuild"
                );
                continue;
            }
            Err(error) => {
                tracing::warn!(
                    community = community_id,
                    new_anchor,
                    ?error,
                    "a successor's new anchor has no verifiable genesis cert; not offered"
                );
                judgements.mark(hash);
                continue;
            }
        };
        judgements.mark(hash);
        match offered_creators(community_id, row, &cert) {
            Ok(creators) => offers.push(GovernanceRuntimeEvent::CommunitySuccessorOffered {
                community_id: community_id.to_owned(),
                new_anchor,
                signer: row.author.clone(),
                reason: row.successor.reason,
                creators,
            }),
            Err(why) => tracing::warn!(
                community = community_id,
                new_anchor,
                ?why,
                "a successor the new community does not link back to is not offered"
            ),
        }
    }
    offers
}

/// Read each pending successor's far cert pinned to its owner and publish
/// [`GovernanceRuntimeEvent::CommunitySuccessorOffered`] for each one both
/// sides link ([`judge`]).
pub async fn announce<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
    state: &GovernanceState,
) {
    let judgements = deps.successor_judgements(community_id);
    let read = |anchor: String| async move {
        crate::genesis::read_pinned(deps, &anchor)
            .await
            .map(|(cert, _)| cert)
    };
    for offer in judge(community_id, state, &judgements, read).await {
        deps.emit_event(offer);
    }
}

#[cfg(test)]
#[path = "successor_tests.rs"]
mod tests;
