//! Our own removal (plan E3.2 S9 own removal), on both modes. When the
//! merged governance names our pseudonym in a `MemberRemoved` or bans it,
//! the Host runs a leave's local side without writing an op: the
//! community's tasks stop (presence, keepalive, catch-up), its watches,
//! gossip mesh and leases go, its keys are forgotten, and it stays as
//! read-only history (its rows and messages are kept, sends are refused).
//! It runs once per session from every projection, the unlock rebuild
//! included, and announces `MembershipEvent::Kicked`/`Banned` naming the
//! remover the first time the removal is seen.

use rekindle_types::subscription_events::{MembershipEvent, SubscriptionEvent};

use crate::host::community::OwnRemoval;

use super::HostGovernance;

/// How long a removed community's tasks get to stop.
const COMMUNITY_STOP_DEADLINE: std::time::Duration = rekindle_types::config::SESSION_STOP_DEADLINE;

impl HostGovernance {
    /// React to `removal` (computed from the state just projected): mark
    /// the community removed, announce it once, tear it down once per
    /// session.
    pub(super) fn on_own_removal(&self, community_id: &str, removal: &OwnRemoval) {
        let (newly, teardown) = self
            .ctx
            .communities
            .note_own_removal(community_id, removal.clone());
        if newly {
            tracing::info!(community = %community_id, banned = removal.banned, "removed from community");
            self.ctx.publish_event(SubscriptionEvent::Membership(
                self.removal_event(community_id, removal),
            ));
        }
        if !teardown {
            return;
        }
        let this = self.clone();
        let community = community_id.to_owned();
        rekindle_governance_runtime::deps::GovernanceRuntimeDeps::scope(self).spawn_or_drop(
            "own removal teardown",
            async move {
                this.tear_down_removed(&community).await;
            },
        );
    }

    /// The event naming us as the removed member and the remover with its
    /// card name.
    fn removal_event(&self, community_id: &str, removal: &OwnRemoval) -> MembershipEvent {
        let community = community_id.to_owned();
        let target_pseudonym = self
            .ctx
            .communities
            .map()
            .read()
            .get(community_id)
            .and_then(|c| c.my_pseudonym_key.clone())
            .unwrap_or_default();
        let removed_by_name = (removal.by.as_ref())
            .and_then(|by| self.ctx.communities.member_card_name(community_id, by));
        let removed_by = removal
            .by
            .as_ref()
            .map(rekindle_types::id::PseudonymKey::to_hex);
        if removal.banned {
            MembershipEvent::Banned {
                community,
                target_pseudonym,
                removed_by,
                removed_by_name,
            }
        } else {
            MembershipEvent::Kicked {
                community,
                target_pseudonym,
                removed_by,
                removed_by_name,
            }
        }
    }

    /// A leave's local side (`create_leave.rs` `leave`) without the
    /// `MemberLeft` op, the departure gossip or forgetting the membership:
    /// stop the tasks, drop the watches and the gossip mesh, release the
    /// leases and forget the keys.
    async fn tear_down_removed(&self, community_id: &str) {
        let daemon_tasks = self.ctx.community_scopes.lock().remove(community_id);
        let store_tasks = self
            .ctx
            .communities
            .map()
            .write()
            .get_mut(community_id)
            .and_then(|c| {
                c.gossip = None;
                c.loops_started = false;
                c.tasks.take()
            });
        for tasks in [daemon_tasks, store_tasks].into_iter().flatten() {
            if let Err(stuck) = tasks.shutdown(COMMUNITY_STOP_DEADLINE).await {
                tracing::warn!(community = %community_id, %stuck, "removed community's tasks did not stop in time");
            }
        }
        let leases = self.ctx.communities.take_leases(community_id);
        let watch_leases = self
            .ctx
            .subscriptions
            .read()
            .as_ref()
            .map(|subs| subs.teardown_community(community_id))
            .unwrap_or_default();
        if let Ok(transport) = self.transport() {
            for lease in leases.all().chain(watch_leases) {
                rekindle_transport::broadcast::dht_writes::release(&transport, lease).await;
            }
        }
        self.ctx.mek_cache.write().remove_community(community_id);
        self.forget_keys(community_id);
        tracing::info!(community = %community_id, "removed community torn down; history kept");
    }
}
