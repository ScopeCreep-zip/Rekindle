//! Our community key's status, published by the Host (FX3 follow-up KP):
//! with the desktop's last-resort mint gone, a member whose key is missing
//! or behind governance's generation waits for a holder's answer to its
//! `RequestMEK` (`mek_recovery.rs`, the sync tick's repair). The wait is
//! state the Host publishes, `CommunityView::key_status` and
//! `GovernanceEvent::KeyStatus` on every change, so no frontend infers it
//! from posts it cannot open (Matrix shows "waiting for this message" on a
//! missing room key the same way; the policy is the Host's).

use rekindle_governance_runtime::GovernanceRuntimeDeps;
use rekindle_types::channel_keys::KeyScope;
use rekindle_types::dto::community::{KeyState, KeyStatus};
use rekindle_types::subscription_events::{GovernanceEvent, SubscriptionEvent};

use super::HostGovernance;
use crate::host::community::CommunityState;

/// `c`'s key status when we hold generation `held` of its community key.
/// A community we were removed from waits for nothing.
pub(crate) fn key_status(c: &CommunityState, held: Option<u64>) -> KeyStatus {
    if c.own_removal.is_some() {
        return KeyStatus::default();
    }
    let need = c.mek_generation;
    let state = if held.is_some_and(|h| h >= need) {
        KeyState::Current
    } else {
        KeyState::Pending
    };
    KeyStatus {
        state,
        have: held,
        need: Some(need),
        since_ms: (state == KeyState::Pending)
            .then_some(c.key_pending_since_ms)
            .flatten(),
    }
}

impl HostGovernance {
    /// The community key generation we hold for `community_id`.
    pub(crate) fn held_community_generation(&self, community_id: &str) -> Option<u64> {
        self.keys()
            .current_epoch(community_id, KeyScope::Community)
            .map(|epoch| epoch.0)
    }

    /// Recompute `community_id`'s key status and publish it when it
    /// changed. Run on every key install, on a merge that raises the
    /// generation and on every community sync tick.
    pub(crate) fn refresh_key_status(&self, community_id: &str) {
        let held = self.held_community_generation(community_id);
        let now = self.ctx.clock.now_ms();
        let status = {
            let mut map = self.ctx.communities.map().write();
            let Some(c) = map.get_mut(community_id) else {
                return;
            };
            let pending = key_status(c, held).state == KeyState::Pending;
            c.key_pending_since_ms = match (pending, c.key_pending_since_ms) {
                (true, None) => Some(now),
                (true, since) => since,
                (false, _) => None,
            };
            let status = key_status(c, held);
            if c.published_key_status.as_ref() == Some(&status) {
                return;
            }
            c.published_key_status = Some(status.clone());
            status
        };
        self.ctx
            .publish_event(SubscriptionEvent::Governance(GovernanceEvent::KeyStatus {
                community: community_id.to_owned(),
                status,
            }));
    }
}
