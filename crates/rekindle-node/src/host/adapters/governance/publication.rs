//! Our own governance ops' publication outcome on the Host (review RV7
//! M2): `Landed` means on our member record AND applied by the converged
//! op set; an own op the convergence refuses or quarantines, then or at
//! any later rebuild, is `Failed` with the reason, and one reinstated
//! later is `Landed` again. The outbox side (`outbox::governance_publication`)
//! announces `Pending` and the row failures.

use rekindle_db::repo::governance_ops::{self as repo, OpState};
use rekindle_types::governance::GovernanceEntry;
use rekindle_types::subscription_events::{GovernanceEvent, OpPublicationState, SubscriptionEvent};

use super::HostGovernance;

/// What the converged op set's verdict on an own op announces, if it is
/// final: `None` while its deps are missing.
pub(super) fn verdict_state(state: OpState) -> Option<OpPublicationState> {
    match state {
        OpState::Applied => Some(OpPublicationState::Landed),
        OpState::PendingDeps => None,
        OpState::Quarantined => Some(OpPublicationState::Failed(
            "the community's converged state refuses it".to_owned(),
        )),
        OpState::Equivocation => Some(OpPublicationState::Failed(
            "our key signed two ops at one seq; the community quarantines its ops".to_owned(),
        )),
    }
}

/// The own ops whose verdict changed from `before` to `after`, with what
/// that announces: a refusal always, a landing only for an op no longer
/// queued (`queued` says which still are; the outbox announces those).
pub(super) fn changed_verdicts(
    before: &[(String, OpState)],
    after: &[(String, OpState)],
    queued: &dyn Fn(&str) -> bool,
) -> Vec<(String, OpPublicationState)> {
    after
        .iter()
        .filter(|(op, state)| {
            before
                .iter()
                .find(|(o, _)| o == op)
                .is_none_or(|(_, prior)| prior != state)
        })
        .filter_map(|(op, state)| {
            let announced = verdict_state(*state)?;
            let landed = announced == OpPublicationState::Landed;
            (!landed || !queued(op)).then(|| (op.clone(), announced))
        })
        .collect()
}

impl HostGovernance {
    /// Publish where our op `op` (its hash, hex) of `community` stands.
    pub(crate) fn announce_op(&self, community: &str, op: &str, state: OpPublicationState) {
        self.ctx.publish_event(SubscriptionEvent::Governance(
            GovernanceEvent::OpPublication {
                community: community.to_owned(),
                op: op.to_owned(),
                state,
            },
        ));
    }

    /// Our op `op` landed on our member record: `Landed` if the converged
    /// op set applies it, `Failed` if it refuses it; nothing while its deps
    /// are missing (the fates announce it once they resolve).
    pub(crate) async fn announce_landed(&self, community: &str, op: &str) {
        let Some(hash) = hex::decode(op)
            .ok()
            .and_then(|b| <[u8; 32]>::try_from(b).ok())
        else {
            return;
        };
        let Some((owner, db)) = self.owner_and_db() else {
            return;
        };
        let cid = community.to_owned();
        let stored = db
            .call(move |conn| repo::get_op(conn, &owner, &cid, &hash))
            .await;
        match stored {
            Ok(Some(stored)) => {
                if let Some(state) = verdict_state(stored.state) {
                    self.announce_op(community, op, state);
                }
            }
            Ok(None) => {}
            Err(error) => tracing::warn!(community, %error, "landed op's verdict not read"),
        }
    }

    /// Announce every own op of `community` whose verdict moved from
    /// `before` to `after` (the fates of a rebuild).
    pub(super) async fn announce_verdicts(
        &self,
        community: &str,
        before: &[(String, OpState)],
        after: &[(String, OpState)],
    ) {
        let mut queued = std::collections::BTreeSet::new();
        if let Some(ops) = self.ctx.outbox.governance_ops() {
            for (op, _) in after {
                if ops.is_queued(op).await.unwrap_or(true) {
                    queued.insert(op.clone());
                }
            }
        }
        for (op, state) in changed_verdicts(before, after, &|op| queued.contains(op)) {
            self.announce_op(community, &op, state);
        }
    }

    /// The revoke-unless-landed row of `invite_id` came due: its
    /// `InviteCreated` landed but the link was never handed out (a crash,
    /// the outbox stopping, or the tombstone's own write failing in the
    /// command). Revoke it unless it is already gone (review RV4 M3).
    ///
    /// # Errors
    /// The revoke was not written; the row stays and is retried.
    pub(crate) async fn revoke_unhanded_invite(
        &self,
        community: &str,
        invite_id: [u8; 16],
    ) -> Result<(), String> {
        let live = self
            .ctx
            .communities
            .governance_state(community)
            .is_some_and(|gov| gov.invites.contains_key(&invite_id));
        if !live {
            return Ok(());
        }
        rekindle_governance_runtime::write_entry_queued(
            self,
            community,
            GovernanceEntry::InviteRevoked { invite_id },
        )
        .await
        .map(|_| ())
        .map_err(|e| e.to_string())
    }
}

#[cfg(test)]
#[path = "publication_tests.rs"]
mod tests;
