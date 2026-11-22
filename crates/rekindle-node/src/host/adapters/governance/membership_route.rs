//! Where the membership control messages enter the Host (plan E3.1 B2e),
//! on both hosts: onboarding answers, a member's own join notice (the
//! gossip peer hint, until E3.4's beat) and its own departure, and the
//! presence tick a community change asks for.

use std::sync::Arc;

use rekindle_codec::community::envelope::ControlPayload;
use rekindle_governance_runtime::membership_events;
use rekindle_governance_runtime::GovernanceRuntimeDeps;

use super::HostGovernance;

impl HostGovernance {
    /// Handle one membership control message from `sender_pseudonym`;
    /// `false` when `payload` is not one.
    pub async fn on_membership_control(
        &self,
        community_id: &str,
        sender_pseudonym: &str,
        payload: &ControlPayload,
    ) -> bool {
        match payload {
            ControlPayload::SubmitOnboardingAnswers { answers } => {
                membership_events::process_onboarding_answers(
                    self,
                    community_id,
                    sender_pseudonym,
                    answers,
                )
                .await;
            }
            ControlPayload::MemberJoined {
                pseudonym_key,
                status,
                route_blob,
                ..
            } if pseudonym_key == sender_pseudonym => {
                self.on_member_joined(community_id, pseudonym_key, status, route_blob.as_deref())
            }
            ControlPayload::MemberLeave { pseudonym_key } if pseudonym_key == sender_pseudonym => {
                self.forget_member(community_id, pseudonym_key).await;
            }
            ControlPayload::MemberJoined { .. } | ControlPayload::MemberLeave { .. } => {
                tracing::debug!(community = %community_id, sender = %sender_pseudonym, "membership announcement about another member dropped");
            }
            _ => return false,
        }
        true
    }

    /// One presence poll right after a community change: the shell runs
    /// its own polls until E3.4; the daemon runs its presence adapter's tick.
    pub(super) fn spawn_presence_poll_tick_impl(&self, community_id: &str) {
        if let Some(shell) = &self.ctx.shell {
            shell.spawn_presence_poll_tick(community_id);
            return;
        }
        let adapter = Arc::new(crate::daemon::presence_adapter::DaemonPresenceAdapter::new(
            Arc::clone(&self.ctx),
        ));
        let community = community_id.to_string();
        self.scope().spawn_or_drop("presence poll tick", async move {
            if let Err(e) = rekindle_presence::community::presence_poll_tick(adapter, &community).await {
                tracing::debug!(community = %community, error = %e, "presence poll tick failed");
            }
        });
    }
}
