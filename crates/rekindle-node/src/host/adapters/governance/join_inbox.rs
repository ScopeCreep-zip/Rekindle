//! Holds, coalesces, and retires our invite-scoped discovery watches.

use std::{
    collections::{BTreeSet, HashMap},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

use rekindle_db::repo::community_invites::{self, JoinInbox};
use rekindle_governance_runtime::{
    join_inbox::{self, Receipt},
    GovernanceRuntimeDeps as _,
};
use rekindle_records::{
    lease::LeaseId,
    port::{OpenIntent, RecordPort, RecordValue},
};
use rekindle_types::{
    dht_layout::join_inbox as layout,
    subscription_events::{GovernanceEvent, SubscriptionEvent},
};
use rekindle_vault::VaultKey;
use tokio::time::Instant;
use zeroize::Zeroizing;

use super::HostGovernance;

const INTERVAL: Duration = Duration::from_secs(60);

#[derive(Debug)]
pub struct Watch {
    inbox: JoinInbox,
    invite_id: [u8; 16],
    community: String,
    code_hash: String,
    lease: LeaseId,
    batch: parking_lot::Mutex<Batch>,
    reading: tokio::sync::Mutex<Observations>,
    jammed: AtomicBool,
}

#[derive(Debug, Default)]
struct Batch {
    pending: BTreeSet<u32>,
    running: bool,
    retired: bool,
    next: Option<Instant>,
}

#[derive(Debug, Default)]
struct Observations {
    values: HashMap<u32, (u32, bool)>,
}

impl Observations {
    fn unavailable(&self, slots: u8) -> bool {
        self.values.values().any(|&(seq, _)| seq >= u32::MAX - 1)
            || (self.values.len() == usize::from(slots)
                && self.values.values().all(|&(_, invalid)| invalid))
    }
}

impl Watch {
    pub(crate) fn lease(&self) -> LeaseId {
        self.lease
    }

    pub(super) fn is_jammed(&self) -> bool {
        self.jammed.load(Ordering::Acquire)
    }

    fn queue(&self, slots: impl IntoIterator<Item = u32>) -> bool {
        let mut batch = self.batch.lock();
        if batch.retired {
            return false;
        }
        let range = layout::slots(self.inbox.slots);
        batch
            .pending
            .extend(slots.into_iter().filter(|slot| range.contains(slot)));
        if batch.running || batch.pending.is_empty() {
            return false;
        }
        batch.running = true;
        true
    }

    fn begin_retirement(&self) -> bool {
        let mut batch = self.batch.lock();
        if batch.retired {
            return false;
        }
        batch.retired = true;
        batch.pending.clear();
        true
    }
}

impl HostGovernance {
    fn join_inbox_keypair(&self, invite_id: &str) -> Option<Zeroizing<String>> {
        let bytes = self
            .ctx
            .vault
            .lock()
            .as_ref()?
            .get(&VaultKey::JoinInboxOwnerKeypair {
                invite_id: invite_id.to_owned(),
            })
            .ok()
            .flatten()?;
        let text = std::str::from_utf8(&bytes).ok()?;
        Some(Zeroizing::new(text.to_owned()))
    }

    pub(super) async fn rearm_join_inboxes(&self, community: &str) {
        let (Ok(owner), Ok(db)) = (self.ctx.require_owner(), self.ctx.db.current()) else {
            return;
        };
        let target = community.to_owned();
        let Ok(ours) = db
            .call(move |c| community_invites::list(c, &owner, &target))
            .await
        else {
            return;
        };
        for own in ours {
            if let Some(inbox) = own.join_inbox {
                self.arm_join_inbox(community, &own.code_hash, inbox).await;
            }
        }
        self.retire_ended_join_inboxes(community);
    }

    pub(super) async fn arm_join_inbox(&self, community: &str, code_hash: &str, inbox: JoinInbox) {
        if !layout::valid_slots(inbox.slots) || !self.holds_community(community) {
            return;
        }
        let Ok(invite_id) = hex::decode(&inbox.invite_id).and_then(|bytes| {
            bytes
                .try_into()
                .map_err(|_| hex::FromHexError::InvalidStringLength)
        }) else {
            return;
        };
        let Some(owner) = self.join_inbox_keypair(&inbox.invite_id) else {
            return;
        };
        let Ok(pool) = self.record_port() else {
            return;
        };
        let existing = self
            .ctx
            .communities
            .map()
            .read()
            .get(community)
            .and_then(|c| c.join_inboxes.get(&inbox.key))
            .cloned();
        if let Some(watch) = existing {
            let slots = layout::slots(watch.inbox.slots);
            self.schedule_join_inbox(watch, slots);
            return;
        }
        let Ok(lease) = pool
            .acquire(&inbox.key, Some(owner.as_str()), OpenIntent::Hold)
            .await
        else {
            return;
        };
        let watch = Arc::new(Watch {
            inbox,
            invite_id,
            community: community.to_owned(),
            code_hash: code_hash.to_owned(),
            lease,
            batch: parking_lot::Mutex::default(),
            reading: tokio::sync::Mutex::default(),
            jammed: AtomicBool::default(),
        });
        let installed = self
            .ctx
            .communities
            .map()
            .write()
            .get_mut(community)
            .is_some_and(|c| {
                use std::collections::btree_map::Entry;
                match c.join_inboxes.entry(watch.inbox.key.clone()) {
                    Entry::Vacant(entry) if c.own_removal.is_none() => {
                        entry.insert(Arc::clone(&watch));
                        true
                    }
                    _ => false,
                }
            });
        if !installed {
            pool.release(lease).await;
            return;
        }
        let slots = layout::slots(watch.inbox.slots).collect::<Vec<_>>();
        if let Err(error) = pool.watch(lease, &slots).await {
            tracing::debug!(community, %error, "join inbox watch will be re-armed by the pool");
        }
        self.schedule_join_inbox(watch, slots);
    }

    pub(super) fn join_inbox_changed(&self, key: &str, slots: &[u32]) -> bool {
        let watch = self
            .ctx
            .communities
            .map()
            .read()
            .values()
            .find_map(|c| c.join_inboxes.get(key).cloned());
        let Some(watch) = watch else {
            return false;
        };
        if slots.is_empty() {
            let range = layout::slots(watch.inbox.slots);
            self.schedule_join_inbox(watch, range);
        } else {
            self.schedule_join_inbox(watch, slots.iter().copied());
        }
        true
    }

    fn schedule_join_inbox(&self, watch: Arc<Watch>, slots: impl IntoIterator<Item = u32>) {
        if !watch.queue(slots) {
            return;
        }
        let Ok(pool) = self.record_port() else {
            watch.batch.lock().running = false;
            return;
        };
        let host = self.clone();
        self.ctx.community_scope(&watch.community).spawn_or_drop(
            "join inbox changes",
            async move {
                loop {
                    let next = {
                        let mut batch = watch.batch.lock();
                        if batch.retired || batch.pending.is_empty() {
                            batch.running = false;
                            return;
                        }
                        batch.next
                    };
                    if let Some(next) = next {
                        tokio::time::sleep_until(next).await;
                    }
                    let slots = {
                        let mut batch = watch.batch.lock();
                        if batch.retired {
                            batch.running = false;
                            return;
                        }
                        batch.next = Some(Instant::now() + INTERVAL);
                        Vec::from_iter(std::mem::take(&mut batch.pending))
                    };
                    host.read_join_inbox(&watch, &*pool, &slots).await;
                }
            },
        );
    }

    async fn read_join_inbox(&self, watch: &Watch, pool: &dyn RecordPort, slots: &[u32]) {
        let mut observations = watch.reading.lock().await;
        let values = match pool.read_hinted(watch.lease, slots).await {
            Ok(values) => values,
            Err(error) => {
                tracing::debug!(community = %watch.community, %error, "join inbox could not be read");
                return;
            }
        };
        for (slot, value) in values {
            if observations
                .values
                .get(&slot)
                .is_some_and(|&(seq, _)| seq >= value.seq)
            {
                continue;
            }
            if let Some(unreadable) = self.receive_join_inbox_value(watch, &value).await {
                observations.values.insert(slot, (value.seq, unreadable));
            }
        }
        let jammed = observations.unavailable(watch.inbox.slots);
        if self.holds_community(&watch.community)
            && watch.jammed.swap(jammed, Ordering::AcqRel) != jammed
        {
            self.ctx.publish_event(SubscriptionEvent::Governance(
                GovernanceEvent::InviteJoinInboxChanged {
                    community: watch.community.clone(),
                    invite_id: watch.inbox.invite_id.clone(),
                },
            ));
        }
    }

    async fn receive_join_inbox_value(&self, watch: &Watch, value: &RecordValue) -> Option<bool> {
        match join_inbox::receive(
            self,
            &watch.community,
            &watch.inbox.key,
            &watch.invite_id,
            &watch.code_hash,
            &value.data,
        )
        .await
        {
            Receipt::Pending => None,
            Receipt::Unreadable => Some(true),
            Receipt::Accepted(joiner) => {
                let deps =
                    crate::daemon::mek_rotation::DaemonMekAdapter::new(Arc::clone(&self.ctx));
                if let Err(error) = self
                    .queue_key_for_joiner(&deps, &watch.community, &joiner)
                    .await
                {
                    tracing::debug!(community = %watch.community, %error, "durable joiner's key could not be queued");
                }
                Some(false)
            }
        }
    }

    pub(super) fn retire_ended_join_inboxes(&self, community: &str) {
        let Some(governance) = self.ctx.communities.governance_state(community) else {
            return;
        };
        let now = self.ctx.clock.now_secs();
        let retired = self
            .ctx
            .communities
            .map()
            .read()
            .get(community)
            .map_or_else(Vec::new, |c| {
                c.join_inboxes
                    .values()
                    .filter(|watch| {
                        watch.inbox.expires_at.is_some_and(|expires| expires <= now)
                            || governance
                                .invites
                                .get(&watch.invite_id)
                                .is_none_or(|invite| invite.uses >= invite.max_uses)
                    })
                    .cloned()
                    .collect()
            });
        let Ok(pool) = self.record_port() else {
            return;
        };
        for watch in retired {
            if !watch.begin_retirement() {
                continue;
            }
            let pool = Arc::clone(&pool);
            let host = self.clone();
            self.ctx
                .community_scope(community)
                .spawn_or_drop("retire join inbox", async move {
                    let slots = layout::slots(watch.inbox.slots).collect::<Vec<_>>();
                    host.read_join_inbox(&watch, &*pool, &slots).await;
                    host.finish_join_inbox_retirement(&watch, &*pool).await;
                });
        }
    }

    /// Forget local owner credentials before a leaving community's
    /// invite rows disappear with the membership.
    pub(super) async fn forget_join_inbox_owners(&self, community: &str) {
        let (Ok(owner), Ok(db)) = (self.ctx.require_owner(), self.ctx.db.current()) else {
            return;
        };
        let target = community.to_owned();
        let Ok(invites) = db
            .call(move |c| community_invites::list(c, &owner, &target))
            .await
        else {
            return;
        };
        if let Some(vault) = self.ctx.vault.lock().as_ref() {
            for inbox in invites.into_iter().filter_map(|invite| invite.join_inbox) {
                if let Err(error) = vault.delete(&VaultKey::JoinInboxOwnerKeypair {
                    invite_id: inbox.invite_id,
                }) {
                    tracing::warn!(community, %error, "join inbox owner could not be erased on leave");
                }
            }
        }
    }

    async fn finish_join_inbox_retirement(&self, watch: &Watch, pool: &dyn RecordPort) {
        let (Ok(owner), Ok(db)) = (self.ctx.require_owner(), self.ctx.db.current()) else {
            return;
        };
        let (community, invite_id) = (watch.community.clone(), watch.inbox.invite_id.clone());
        let erased = self.ctx.vault.lock().as_ref().is_some_and(|vault| {
            vault
                .delete(&VaultKey::JoinInboxOwnerKeypair {
                    invite_id: invite_id.clone(),
                })
                .is_ok()
        });
        if !erased {
            tracing::warn!(%community, %invite_id, "retired join inbox owner could not be erased");
            watch.batch.lock().retired = false;
            return;
        }
        if let Err(error) = db
            .call(move |c| community_invites::retire_inbox(c, &owner, &community, &invite_id))
            .await
        {
            tracing::warn!(community = %watch.community, %error, "retired join inbox row could not be removed");
        }
        if let Some(c) = self.ctx.communities.map().write().get_mut(&watch.community) {
            c.join_inboxes.remove(&watch.inbox.key);
        }
        pool.release(watch.lease).await;
    }
}

#[cfg(test)]
#[path = "join_inbox_tests.rs"]
mod tests;
