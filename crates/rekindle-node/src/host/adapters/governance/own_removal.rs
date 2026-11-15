//! Our own removal (plan E3.2 S9 own removal), on both modes. When the
//! merged governance names our pseudonym in a `MemberRemoved` or bans it,
//! the Host runs a leave's local side without writing an op: the
//! community's tasks stop (presence, keepalive, catch-up), its watches,
//! gossip mesh and leases go, its keys leave memory, and it stays as
//! read-only history (its rows and messages are kept, sends are refused).
//! It follows the merged state of every projection, the unlock rebuild
//! included, and announces `MembershipEvent::Kicked`/`Banned` naming the
//! remover when the removal is first seen.
//!
//! A removal is not final when it is merged: under strong removal
//! (`rekindle_governance::merge`, p2panda-auth) a removal concurrent with
//! the remover's own removal is void, and the op that voids it can arrive
//! after it. So the mark follows the current merged state (a later state
//! that no longer removes us clears it and restores the community), and
//! the teardown keeps what a restore needs: the vault's keys (the slot
//! keypair, the member HPKE secret, the MEKs) stay, only the in-memory
//! caches are dropped. A real removal is followed by the key rotation
//! (ADR 0018), which is what keeps us from later content.

use rekindle_types::subscription_events::{MembershipEvent, SubscriptionEvent};

use crate::host::community::OwnRemoval;

use super::HostGovernance;

/// How long a removed community's tasks get to stop.
const COMMUNITY_STOP_DEADLINE: std::time::Duration = rekindle_types::config::SESSION_STOP_DEADLINE;

impl HostGovernance {
    /// React to `removal` (computed from the state just projected): mark
    /// the community removed, announce it the first time, and bring the
    /// network side in line.
    pub(super) fn on_own_removal(&self, community_id: &str, removal: &OwnRemoval) {
        if self
            .ctx
            .communities
            .note_own_removal(community_id, removal.clone())
        {
            tracing::info!(community = %community_id, banned = removal.banned, "removed from community");
            self.ctx.publish_event(SubscriptionEvent::Membership(
                self.removal_event(community_id, removal),
            ));
        }
        self.spawn_reconcile(community_id);
    }

    /// The state just projected does not remove us: a removal marked
    /// before was void (strong removal), so the mark goes and the
    /// community is restored.
    pub(super) fn on_not_removed(&self, community_id: &str) {
        if self.ctx.communities.clear_own_removal(community_id) {
            tracing::info!(community = %community_id, "our removal was void; community restored");
            self.spawn_reconcile(community_id);
        }
    }

    /// Bring the community's network side in line with the mark, under its
    /// removal gate, so a teardown and a restore never interleave.
    fn spawn_reconcile(&self, community_id: &str) {
        let this = self.clone();
        let community = community_id.to_owned();
        rekindle_governance_runtime::deps::GovernanceRuntimeDeps::scope(self).spawn_or_drop(
            "own removal reconcile",
            async move {
                this.reconcile_removal(&community).await;
            },
        );
    }

    /// Tear down a removed community that is still up, restore a torn-down
    /// one whose removal is gone; otherwise nothing.
    pub(super) async fn reconcile_removal(&self, community_id: &str) {
        let Some(gate) = self.ctx.communities.removal_gate(community_id) else {
            return;
        };
        let mut torn_down = gate.lock().await;
        let removed = self.ctx.communities.own_removal(community_id).is_some();
        if removed && !*torn_down {
            self.tear_down_removed(community_id).await;
            *torn_down = true;
        } else if !removed && *torn_down {
            self.restore_unremoved(community_id).await;
            *torn_down = false;
        }
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
    /// `MemberLeft` op, the departure gossip, forgetting the membership or
    /// deleting the vault's keys: stop the tasks, drop the watches and the
    /// gossip mesh, release the leases and drop the in-memory keys.
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
        if let Some(shell) = &self.ctx.shell {
            shell.community_left(community_id);
        }
        tracing::info!(community = %community_id, "removed community torn down; history kept");
    }

    /// Undo [`Self::tear_down_removed`] for a removal that turned out void:
    /// the vault's keys back into the caches, then the community brought up
    /// as the login stage brings each one up (records, watches, keepalive,
    /// hydration, rebuild, catch-up) and, on the daemon, as a join activates
    /// it (subscription and gossip mesh, presence).
    async fn restore_unremoved(&self, community_id: &str) {
        self.restore_stored_meks();
        super::login_stage::bring_up(self, community_id).await;
        if self.ctx.shell.is_none() {
            let membership =
                (self.ctx.session.read().as_ref()).and_then(|s| s.community(community_id).cloned());
            if let Some(membership) = membership {
                crate::daemon::dispatch::community::activate_community(&self.ctx, &membership)
                    .await;
            }
        }
        tracing::info!(community = %community_id, "community restored after a void removal");
    }
}
