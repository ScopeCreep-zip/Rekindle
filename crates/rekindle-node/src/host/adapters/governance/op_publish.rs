//! Publishing our signed governance op on the `governance_op` outbox
//! stream (plan E3.2 S6b, D22): the row is stored before the op is
//! gossiped or installed (lead ruling 2026-11-26, item 2), and the
//! returned [`OpPublication`](rekindle_governance_runtime::OpPublication)
//! is the wait for what its landing names.

use rekindle_governance_runtime::GovernanceRuntimeError;

use super::HostGovernance;

fn adapter(context: &str, e: impl std::fmt::Display) -> GovernanceRuntimeError {
    GovernanceRuntimeError::Adapter(format!("{context}: {e}"))
}

impl HostGovernance {
    /// Publish `encoded` on our device's op record (ADR 0022 rules 3 and
    /// 10, `device_signer::encoded_op_record`, the write path's rule: a
    /// `MemberJoined` on the record its carried endorsement binds, else
    /// `M(p)` for the first device or the `M(p,d)` our key's endorsement
    /// binds in the state) and wait for what
    /// `landing` names: its first attempt (a queued op stands; community
    /// create only), its landing on `M`/2 and `M`/3 within the command's
    /// deadline, or only its row on the stream.
    pub(super) async fn publish_governance_op_impl(
        &self,
        community_id: &str,
        encoded: Vec<u8>,
        landing: rekindle_governance_runtime::OpLanding,
    ) -> Result<rekindle_governance_runtime::OpPublication, GovernanceRuntimeError> {
        let state = self.governance_state_impl(community_id).unwrap_or_default();
        let record = rekindle_governance_runtime::device_signer::encoded_op_record(
            self,
            community_id,
            &state,
            &encoded,
        )?;
        let ops = self
            .ctx
            .outbox
            .governance_ops()
            .ok_or_else(|| adapter("publish op", "the outbox is not running"))?;
        let community = community_id.to_owned();
        // Each arm returns once the row is stored (the caller gossips only
        // then, lead ruling 2026-11-26 item 2); the publication is the wait.
        let publication = match landing {
            rekindle_governance_runtime::OpLanding::FirstAttempt => {
                let wait = ops
                    .publish_answer(
                        community_id,
                        &record,
                        encoded,
                        rekindle_types::config::GOVERNANCE_ANSWER_BOUND,
                    )
                    .await
                    .map_err(|e| adapter("publish op", e))?;
                rekindle_governance_runtime::OpPublication::new(async move {
                    let published = wait.await.map_err(|e| adapter("publish op", e))?;
                    if let crate::host::adapters::outbox::Published::Queued(reason) = &published {
                        tracing::info!(community = %community, %reason, "governance op pending, queued on the outbox");
                    }
                    Ok(super::ops::outcome_of(&published))
                })
            }
            // Bounded by the command's deadline (the client's
            // `NETWORK_COMMAND_DEADLINE`, answered before it; shared with the
            // barrier's wait in line): the link, with its revoke row (RV4 M3).
            rekindle_governance_runtime::OpLanding::InviteLanded { invite_id, .. } => {
                let wait = ops
                    .publish_invite_created(
                        community_id,
                        &record,
                        encoded,
                        invite_id,
                        landing.remaining(),
                    )
                    .await
                    .map_err(|e| adapter("publish op", e))?;
                rekindle_governance_runtime::OpPublication::new(async move {
                    let published = wait.await.map_err(|e| adapter("publish op", e))?;
                    // The command is refused, but the op stands (it is in
                    // our state, gossiped, and the outbox lands it).
                    if let crate::host::adapters::outbox::Published::Queued(reason) = published {
                        tracing::warn!(community = %community, %reason, "governance op not yet published, queued");
                        return Err(GovernanceRuntimeError::NotYetPublished);
                    }
                    Ok(rekindle_governance_runtime::OpOutcome::Landed)
                })
            }
            rekindle_governance_runtime::OpLanding::Queued => {
                ops.queue(community_id, &record, encoded)
                    .await
                    .map_err(|e| adapter("queue op", e))?;
                rekindle_governance_runtime::OpPublication::settled(Ok(
                    rekindle_governance_runtime::OpOutcome::Pending,
                ))
            }
        };
        Ok(publication)
    }
}
