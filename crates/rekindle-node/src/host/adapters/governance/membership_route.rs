//! Where the membership control messages enter the Host (plan E3.1 B2e),
//! on both hosts: onboarding answers, a member's own join notice (the
//! gossip peer hint, until E3.4's beat) and its own departure.

use rekindle_codec::community::envelope::ControlPayload;
use rekindle_governance_runtime::membership_events;

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
                self.on_member_joined(community_id, pseudonym_key, status, route_blob.as_deref());
            }
            ControlPayload::MemberLeave { pseudonym_key } if pseudonym_key == sender_pseudonym => {
                self.forget_member(community_id, pseudonym_key).await;
            }
            ControlPayload::MemberJoined { .. } | ControlPayload::MemberLeave { .. } => {
                tracing::debug!(community = %community_id, sender = %sender_pseudonym, "membership announcement about another member dropped");
            }
            ControlPayload::Kick { target_pseudonym } => {
                self.on_relayed_kick(community_id, target_pseudonym, sender_pseudonym)
                    .await;
            }
            _ => return false,
        }
        true
    }

    /// A relayed `Kick` of `target` from `kicker` (plan D20, GI3), the one
    /// path on both hosts. Gossip is unauthenticated as to authority, so it
    /// is acted on only when `kicker` holds KICK_MEMBERS under the merged
    /// governance; our own removal is the merge's (plan E3.2 S9), never a
    /// notice's. An authorized kick forgets the member here and starts the
    /// departure rotation: the shell's MEK stage until the MEK adapters move
    /// (E3.5), the rotation worker on the daemon. The membership event is
    /// the merge's to publish, once, with its remover and reason
    /// (`member_events.rs`), never the notice's.
    pub async fn on_relayed_kick(&self, community_id: &str, target: &str, kicker: &str) {
        let me = self.ctx.my_pseudonym(community_id);
        if self.ctx.communities.own_removal(community_id).is_some()
            || (!me.is_empty() && target.eq_ignore_ascii_case(&me))
        {
            return;
        }
        if !crate::daemon::mek_rotation::may_kick(&self.ctx, community_id, kicker) {
            tracing::debug!(community = %community_id, sender = %kicker, "kick from a member without KICK_MEMBERS dropped");
            return;
        }
        self.forget_member(community_id, target).await;
        if let Some(shell) = &self.ctx.shell {
            shell.spawn_departure_rotation(community_id, target);
            return;
        }
        let request =
            crate::daemon::mek_rotation::MekRotationRequest::kick(community_id, target, kicker);
        if self.ctx.mek_rotation_tx.send(request).is_err() {
            tracing::debug!(community = %community_id, "MEK rotation worker stopped — kick not rotated");
        }
    }
}
