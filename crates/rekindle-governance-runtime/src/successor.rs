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

use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;

use rekindle_governance::state::GovernanceState;
use rekindle_governance::successor::{offered_creators, SuccessorState};
use rekindle_types::governance::{
    CommunitySuccessor, GenesisCert, GovernanceEntry, OpHash, SuccessorReason,
};
use rekindle_types::id::PseudonymKey;

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
    /// The offers among them, for a window opened after the event (lead
    /// decision (33)'s snapshot).
    offered: parking_lot::Mutex<BTreeMap<OpHash, GovernanceRuntimeEvent>>,
}

impl SuccessorJudgements {
    fn is_judged(&self, hash: &OpHash) -> bool {
        self.judged.lock().contains(hash)
    }

    fn mark(&self, hash: OpHash) {
        self.judged.lock().insert(hash);
    }

    fn offer(&self, hash: OpHash, offer: GovernanceRuntimeEvent) {
        self.offered.lock().insert(hash, offer);
    }

    /// Every successor offered this session, each a
    /// [`GovernanceRuntimeEvent::CommunitySuccessorOffered`], in op-hash
    /// order.
    #[must_use]
    pub fn offers(&self) -> Vec<GovernanceRuntimeEvent> {
        self.offered.lock().values().cloned().collect()
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
            Ok(creators) => {
                let offer = GovernanceRuntimeEvent::CommunitySuccessorOffered {
                    community_id: community_id.to_owned(),
                    new_anchor,
                    signer: row.author.clone(),
                    reason: row.successor.reason,
                    creators,
                };
                judgements.offer(hash, offer.clone());
                offers.push(offer);
            }
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

/// Re-create `community_id` under a new anchor whose cert names it as
/// `predecessor` (plan E3.2 S7s.4, `publish_community_successor`): the
/// same name and admission mode, with us as its sole creator. Returns the
/// new anchor; [`publish`] then names it here.
///
/// # Errors
/// We are not a creator here (ADR 0011: only a creator signs a successor),
/// the state is not loaded, or the create failed.
pub async fn create_successor<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
) -> Result<String, GovernanceRuntimeError> {
    let state = deps
        .governance_state(community_id)
        .ok_or_else(|| GovernanceRuntimeError::GovernanceStateMissing(community_id.to_owned()))?;
    let me = deps
        .community_membership(community_id)
        .and_then(|m| m.my_pseudonym_hex)
        .ok_or_else(|| GovernanceRuntimeError::PseudonymKeyMissing(community_id.to_owned()))?;
    let me_key = PseudonymKey::from_hex_lossy(&me);
    if !rekindle_governance::permissions::holds_creator_seat(&me_key, &state) {
        return Err(GovernanceRuntimeError::PermissionDenied);
    }
    let name = (state.metadata.as_ref())
        .map(|m| m.name.clone())
        .ok_or_else(|| GovernanceRuntimeError::GovernanceStateMissing(community_id.to_owned()))?;
    let admission = state.effective_admission_mode();
    crate::origin::create_successor_community(deps, &name, admission, community_id).await
}

/// Name `new_anchor` as `community_id`'s successor: the near link, the
/// `CommunitySuccessor` op readers check against the new cert's
/// `predecessor` (ADR 0011), carrying `invites_record`, the first page of
/// the continuing members' sealed invites (lead decision (34),
/// [`crate::successor_invites`]).
///
/// # Errors
/// The op was refused or not written.
pub async fn publish<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
    new_anchor: &str,
    invites_record: &str,
) -> Result<(), GovernanceRuntimeError> {
    let entry = successor_entry(new_anchor, invites_record);
    crate::apply::write_entry(deps, community_id, entry)
        .await
        .map(drop)
}

/// The re-creation `CommunitySuccessor` op [`publish`] writes: it names
/// `new_anchor` and carries `invites_record`.
fn successor_entry(new_anchor: &str, invites_record: &str) -> GovernanceEntry {
    GovernanceEntry::CommunitySuccessor(CommunitySuccessor {
        new_anchor: Some(new_anchor.to_owned()),
        reason: SuccessorReason::Recreate,
        transfer_to: None,
        creator_set_hash: None,
        accepts: None,
        links: Vec::new(),
        invites_record: Some(invites_record.to_owned()),
    })
}

/// The signer and creator set of an applied successor of `community`
/// naming `new_anchor`, judged against `cert`, the new anchor's verified
/// cert: both links from the new community's side, so a follow never
/// trusts an offer it has not re-checked. The first row both links hold
/// wins; none → the last refusal, or `NoSuccessorOp`.
///
/// # Errors
/// No applied successor here names `new_anchor`, or its links fail.
pub fn follow_creators(
    community: &str,
    state: &GovernanceState,
    new_anchor: &str,
    cert: &GenesisCert,
) -> Result<(PseudonymKey, Vec<PseudonymKey>), FollowRefusal> {
    let mut refusal = FollowRefusal::NoSuccessorOp;
    let rows = (state.successors.values())
        .filter(|row| row.successor.new_anchor.as_deref() == Some(new_anchor));
    for row in rows {
        match offered_creators(community, row, cert) {
            Ok(creators) => return Ok((row.author.clone(), creators)),
            Err(why) => refusal = FollowRefusal::Links(why),
        }
    }
    Err(refusal)
}

/// Why a successor cannot be followed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FollowRefusal {
    /// No applied successor in the old community names the anchor (the
    /// near link is missing).
    NoSuccessorOp,
    /// The new anchor's cert does not link back.
    Links(rekindle_governance::successor::SuccessorRefusal),
}

/// Check both links before following `new_anchor` from `community_id`:
/// read the new cert pinned to its owner and judge it with
/// [`follow_creators`]. Returns the linked successor's `invitesRecord`,
/// where the follower's sealed invite is (lead decision (34)).
///
/// # Errors
/// The state is not loaded, the cert cannot be read, a link fails, or the
/// linked successor names no invites record.
pub async fn check_follow<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
    new_anchor: &str,
) -> Result<String, GovernanceRuntimeError> {
    let state = deps
        .governance_state(community_id)
        .ok_or_else(|| GovernanceRuntimeError::GovernanceStateMissing(community_id.to_owned()))?;
    let (cert, _) = crate::genesis::read_pinned(deps, new_anchor).await?;
    let (signer, _) = follow_creators(community_id, &state, new_anchor, &cert).map_err(|why| {
        GovernanceRuntimeError::OpRefused(format!("not a linked successor: {why:?}"))
    })?;
    invites_record(&state, new_anchor, &signer).ok_or_else(|| {
        GovernanceRuntimeError::OpRefused(format!("the successor {new_anchor} names no invites"))
    })
}

/// The `invitesRecord` of `signer`'s applied successor naming `new_anchor`.
#[must_use]
pub fn invites_record(
    state: &GovernanceState,
    new_anchor: &str,
    signer: &PseudonymKey,
) -> Option<String> {
    (state.successors.values())
        .filter(|row| row.author == *signer)
        .filter(|row| row.successor.new_anchor.as_deref() == Some(new_anchor))
        .find_map(|row| row.successor.invites_record.clone())
}

#[cfg(test)]
#[path = "successor_tests.rs"]
mod tests;
