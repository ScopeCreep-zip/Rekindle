//! The onboarding answers on both hosts (plan E3.1 B2e): the
//! `MembershipEventDeps` the governance runtime's processor
//! (`rekindle_governance_runtime::membership_events`) runs over the Host's
//! community store, session and database. `membership_route` routes the
//! control messages here.

use rekindle_governance_runtime::membership_events::MembershipEventDeps;

use super::HostGovernance;

impl HostGovernance {
    /// The unlocked identity's key and database, or `None` while locked.
    pub(super) fn owner_and_db(&self) -> Option<(String, rekindle_db::Db)> {
        let owner = self
            .ctx
            .session
            .read()
            .as_ref()
            .map(|s| s.identity.public_key_hex.clone())?;
        Some((owner, self.ctx.db.current().ok()?))
    }
}

impl MembershipEventDeps for HostGovernance {
    fn persist_onboarding_completion(
        &self,
        community_id: &str,
        pseudonym_hex: &str,
        is_self: bool,
    ) {
        self.ctx
            .communities
            .note_onboarding_completion(community_id, is_self);
        let Some((owner, db)) = self.owner_and_db() else {
            return;
        };
        let (community, pseudonym) = (community_id.to_string(), pseudonym_hex.to_string());
        db.fire("onboarding completion", move |conn| {
            rekindle_db::repo::members::set_onboarding_complete(
                conn, &owner, &community, &pseudonym,
            )
        });
    }

    fn push_onboarding_complete_to_sync(&self, community_id: &str) {
        if let Some(shell) = &self.ctx.shell {
            shell.push_onboarding_complete_to_sync(community_id);
        } else {
            tracing::debug!(
                community_id,
                "onboarding completion: no personal sync record on the daemon track (E5.2)"
            );
        }
    }
}
