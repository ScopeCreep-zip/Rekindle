//! The `ReEnrolDevice` review and its commit on the Host (ADR 0022 rules 4
//! and 9, plan E3.2 S7d-keys.10). The review is raised by the Host itself
//! when the unlock barrier finds a restore, when the own-record watch
//! finds our key retired (`OwnKeyRetired`), and at every unlock while the
//! caught-up set still retires our key, so it comes back until committed;
//! a committed re-enrolment leaves a fresh key whose barrier passes and
//! which no `DeviceRetired` names, so nothing raises it again. The commit
//! writes one `DeviceReEnrol` in every community this data root holds.
//! The same commit replaces the prekey record and device id, renews each
//! card's HPKE secret, and queues the contacts' session reset prompts.
//! E5.3 adds the device root key and the DeviceList entry to what it
//! replaces.

use rekindle_governance_runtime::{device_re_enrol, GovernanceRuntimeError, OpOutcome};
use rekindle_ipc::protocol::{CommunityRequest, IpcRequest, LifecycleRequest};
use rekindle_types::intent::IntentKind;

use super::HostGovernance;

/// The request the review commits.
fn re_enrol_request() -> IpcRequest {
    IpcRequest::Community(CommunityRequest::Lifecycle(
        LifecycleRequest::CommunityDeviceReEnrol {},
    ))
}

/// One `ReEnrolDevice` hold at a time: a newer review replaces the older.
fn same_review(a: &IpcRequest, b: &IpcRequest) -> bool {
    let review = |r: &IpcRequest| {
        matches!(
            r,
            IpcRequest::Community(CommunityRequest::Lifecycle(
                LifecycleRequest::CommunityDeviceReEnrol {}
            ))
        )
    };
    review(a) && review(b)
}

impl HostGovernance {
    /// Raise the backend-originated `ReEnrolDevice` review, found in
    /// `community`.
    pub(super) async fn raise_re_enrol(&self, community: &str) {
        let raised = crate::host::backend_hold::raise(
            &self.ctx,
            re_enrol_request(),
            None,
            IntentKind::ReEnrolDevice,
            same_review,
        )
        .await;
        match raised {
            Ok(()) => tracing::info!(community, "device re-enrolment review raised"),
            Err(refused) => {
                tracing::warn!(community, ?refused, "device re-enrolment review not raised");
            }
        }
    }

    /// [`Self::raise_re_enrol`] on `community`'s scope (from a sync path).
    pub(super) fn spawn_re_enrol_review(&self, community: &str) {
        let governance = self.clone();
        let found = community.to_owned();
        self.ctx.community_scope(community).spawn_or_drop(
            "device re-enrolment review",
            async move {
                governance.raise_re_enrol(&found).await;
            },
        );
    }

    /// Whether this device must re-enrol in `community`.
    pub(super) fn needs_re_enrol(&self, community: &str) -> bool {
        match device_re_enrol::needed(self, community) {
            Ok(needed) => needed,
            Err(error) => {
                tracing::debug!(community, %error, "re-enrolment check skipped");
                false
            }
        }
    }

    /// The communities we hold and were not removed from, in id order.
    fn sorted_held_communities(&self) -> Vec<String> {
        let mut held = self.held_communities();
        held.sort();
        held
    }

    /// The review's `lastSeq`: the lowest over the communities that found
    /// a restore or a retirement of our key; `None` when none did.
    pub(crate) async fn re_enrol_last_seq(&self) -> Option<u64> {
        let mut lowest = None;
        for community in self.sorted_held_communities() {
            if !self.needs_re_enrol(&community) {
                continue;
            }
            match device_re_enrol::last_seq(self, &community).await {
                Ok(seq) => lowest = Some(lowest.map_or(seq, |l: u64| l.min(seq))),
                Err(error) => tracing::debug!(community, %error, "re-enrolment seq not read"),
            }
        }
        lowest
    }

    /// The commit: one `DeviceReEnrol` in every community this data root
    /// holds (a copied data root copied every community's key, ADR 0022
    /// rule 4), each answered.
    pub(crate) async fn re_enrol_everywhere(
        &self,
    ) -> Result<Vec<(String, Result<OpOutcome, GovernanceRuntimeError>)>, String> {
        let _re_enrol = self.ctx.device_re_enrol.lock().await;
        let communities = self.sorted_held_communities();
        if !communities
            .iter()
            .any(|community| self.needs_re_enrol(community))
        {
            return Ok(Vec::new());
        }
        crate::host::own_prekeys::replace_for_re_enrol(&self.ctx).await?;
        let mut written = Vec::new();
        for community in communities {
            let outcome = match self.stage_retirement_owner(&community) {
                Ok(()) => Box::pin(device_re_enrol::re_enrol(self, &community)).await,
                Err(error) => Err(error),
            };
            if outcome.is_ok() {
                self.persist_governance_snapshot(&community).await;
                if let Err(error) = self.replace_card_secret(&community).await {
                    tracing::warn!(community, %error, "re-enrolment could not update the member card");
                }
            }
            written.push((community, outcome));
        }
        self.prompt_session_resets().await?;
        if written.iter().all(|(_, result)| result.is_ok()) {
            self.ctx.intents.retain_held(|owner, action| {
                owner.conn.is_some() || !same_review(action, &re_enrol_request())
            });
        }
        Ok(written)
    }

    async fn prompt_session_resets(&self) -> Result<(), String> {
        use rekindle_types::friend_store::FriendshipState;
        let owner = self
            .ctx
            .require_owner()
            .map_err(|_| "the identity is locked")?;
        let db = self.ctx.db.current().map_err(|e| e.to_string())?;
        let contacts = db
            .call(move |conn| rekindle_db::repo::friends::load_all(conn, &owner))
            .await
            .map_err(|e| e.to_string())?;
        let host = crate::host::adapters::friends::HostFriends::of(&self.ctx);
        for contact in contacts
            .into_iter()
            .filter(|c| c.friendship_state == FriendshipState::Accepted)
        {
            if let Err(error) = rekindle_transport::operations::friend::session_reset::start(
                &host,
                &contact.public_key,
            )
            .await
            {
                tracing::warn!(%error, peer = %contact.public_key, "re-enrolment session prompt not queued");
            }
        }
        Ok(())
    }
}
