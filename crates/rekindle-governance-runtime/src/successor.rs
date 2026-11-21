//! The successor reader (ADR 0011 "Successor community", plan E3.2 S7s.3e
//! part 2, lead decision (24)): an applied `CommunitySuccessor` naming a
//! new anchor is offered to the members only once the far side links back.
//! The reader fetches the new anchor's `GenesisCert` pinned to its owner
//! ([`crate::genesis::read_pinned`]) and judges it
//! ([`offered_creators`]: `predecessor` names this community and, for a
//! transfer acceptance, the creator links hold and are the cert's creator
//! set). Anything else is refused, never offered. It runs after each merge
//! of the rebuild (`op_log::rebuild_ops`), for the ops that merge newly
//! applies, so each successor is offered once.

use rekindle_governance::state::GovernanceState;
use rekindle_governance::successor::{offered_creators, SuccessorState};

use crate::deps::GovernanceRuntimeDeps;
use crate::event::GovernanceRuntimeEvent;
use crate::op_log::{HeldOp, OpFate};

/// The applied successors and acceptances (the rows naming a new anchor)
/// this merge newly applies: each held under another fate before it, so
/// one already announced stays quiet. Two valid successors are both kept.
#[must_use]
pub fn newly_applied<'a>(held: &[HeldOp], state: &'a GovernanceState) -> Vec<&'a SuccessorState> {
    (state.successors.iter())
        .filter(|(hash, row)| {
            row.successor.new_anchor.is_some()
                && held
                    .iter()
                    .any(|h| h.hash == **hash && h.fate != OpFate::Applied)
        })
        .map(|(_, row)| row)
        .collect()
}

/// Read each newly applied successor's far cert and publish
/// [`GovernanceRuntimeEvent::CommunitySuccessorOffered`] for each one both
/// sides link; one whose cert is unreadable or does not link back is
/// refused with a WARN.
pub async fn announce<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
    held: &[HeldOp],
    state: &GovernanceState,
) {
    for row in newly_applied(held, state) {
        let Some(new_anchor) = row.successor.new_anchor.clone() else {
            continue;
        };
        let cert = match crate::genesis::read_pinned(deps, &new_anchor).await {
            Ok((cert, _)) => cert,
            Err(error) => {
                tracing::warn!(
                    community = community_id,
                    new_anchor,
                    ?error,
                    "a successor's new anchor has no verifiable genesis cert; not offered"
                );
                continue;
            }
        };
        match offered_creators(community_id, row, &cert) {
            Ok(creators) => deps.emit_event(GovernanceRuntimeEvent::CommunitySuccessorOffered {
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
}

#[cfg(test)]
#[path = "successor_tests.rs"]
mod tests;
