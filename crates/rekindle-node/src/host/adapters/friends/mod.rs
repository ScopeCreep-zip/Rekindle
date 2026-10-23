//! The friend and 1:1 operations' Host (plan E2.1): one implementation of
//! `rekindle_transport::operations::friend::FriendHost` over the
//! [`DaemonContext`], used by both hosts. The friend presence adapter
//! (`presence`) and the maintenance task (`maintenance`) run over it too.

mod inbox_watch;
mod maintenance;
mod participant;
mod presence;
mod watch;

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::{Mutex, RwLock};
use rekindle_ipc::protocol::IpcRequest;
use rekindle_records::lease::LeaseId;
use rekindle_transport::operations::friend::watch_retry::WatchTarget;
use rekindle_transport::operations::friend::{BackendHold, FriendError, FriendHost, FriendStores};
use rekindle_transport::{Session, TransportNode};
use rekindle_types::friend_store::FriendStore;
use rekindle_types::intent::IntentKind;
use rekindle_types::subscription_events::SubscriptionEvent;
use zeroize::Zeroizing;

use crate::daemon::dispatch::DaemonContext;
use crate::host::gate::IntentOwner;

pub(crate) use maintenance::{
    start as start_maintenance, start_prekeys as start_prekey_maintenance,
};
pub(crate) use watch::watch_friends;

/// Each watched friend's profile and participant-record leases (a watch
/// that failed is re-opened when the network comes back).
#[derive(Default)]
pub struct FriendWatches {
    leases: Mutex<HashMap<String, LeaseId>>,
    /// Each open invite's request inbox lease, by record key (plan E2.5).
    inboxes: Mutex<HashMap<String, LeaseId>>,
    /// Each friend's participant record (key, lease), by friend (plan E2.5).
    participants: Mutex<HashMap<String, (String, LeaseId)>>,
    /// Whether the network was ready at its last change.
    network_ready: std::sync::atomic::AtomicBool,
}

/// The Host, as the friend operations see it.
#[derive(Clone)]
pub struct HostFriends {
    ctx: Arc<DaemonContext>,
}

impl HostFriends {
    /// The Host over `ctx`.
    pub(crate) fn of(ctx: &DaemonContext) -> Self {
        Self {
            ctx: ctx.this.upgrade().expect("a context in use is alive"),
        }
    }

    pub(crate) fn ctx(&self) -> &DaemonContext {
        &self.ctx
    }
}

#[async_trait::async_trait]
impl FriendHost for HostFriends {
    fn owned(&self) -> Arc<dyn FriendHost> {
        Arc::new(self.clone())
    }

    fn db(&self) -> Result<rekindle_db::Db, FriendError> {
        self.ctx.db.current().map_err(|_| FriendError::Locked)
    }

    fn session(&self) -> &Arc<RwLock<Option<Session>>> {
        &self.ctx.session
    }

    fn local_party(&self) -> Result<rekindle_db::ratchet::LocalParty, FriendError> {
        let secret = self.identity_secret()?;
        Ok(rekindle_db::ratchet::LocalParty {
            identity: Arc::new(rekindle_crypto::Identity::from_secret_bytes(&secret)),
            // Its draws run on the database's thread: a fork, seeded here in
            // this task's order, keeps a seeded Host's trace fixed.
            rng: Arc::new(rekindle_secrets::forked_random::ForkedRandom::fork(
                self.ctx.rng.as_ref(),
            )),
            now_ms: self.ctx.clock.now_ms_i64(),
        })
    }

    fn identity_secret(&self) -> Result<Zeroizing<[u8; 32]>, FriendError> {
        self.ctx
            .identity_secret()
            .map(Zeroizing::new)
            .ok_or(FriendError::Locked)
    }

    fn node(&self) -> Result<Arc<TransportNode>, FriendError> {
        self.ctx.transport.read().clone().ok_or_else(|| {
            FriendError::NotReady("transport not started — the network is not up yet".to_string())
        })
    }

    fn friend_store(&self) -> Result<Arc<dyn FriendStore>, FriendError> {
        self.ctx
            .friend_store
            .read()
            .clone()
            .map(|store| store as Arc<dyn FriendStore>)
            .ok_or(FriendError::Locked)
    }

    fn stores(&self) -> &FriendStores {
        &self.ctx.friend_stores
    }

    fn outbox(
        &self,
    ) -> Result<Arc<dyn rekindle_transport::operations::dm::outbox::DmOutbox>, FriendError> {
        self.ctx.outbox.dm().ok_or(FriendError::Locked)
    }

    async fn participant_writer(
        &self,
        peer: &str,
    ) -> Result<rekindle_transport::operations::dm::stream::ParticipantWriter, FriendError> {
        participant::writer(self, peer).await
    }

    async fn held_participant_writer(
        &self,
        peer: &str,
    ) -> Result<Option<rekindle_transport::operations::dm::stream::ParticipantWriter>, FriendError>
    {
        participant::held(self, peer).await
    }

    fn group_dm(&self) -> Result<Arc<dyn rekindle_dm::GroupDmDeps>, FriendError> {
        super::group_dm::open(self)
    }

    async fn group_chain_writer(
        &self,
        group_id: &str,
    ) -> Result<rekindle_transport::operations::dm::stream::ParticipantWriter, FriendError> {
        super::group_dm::chain::writer(self, group_id).await
    }

    async fn held_group_chain_writer(
        &self,
        group_id: &str,
    ) -> Result<Option<rekindle_transport::operations::dm::stream::ParticipantWriter>, FriendError>
    {
        super::group_dm::chain::held(self, group_id)
    }

    fn invisible(&self) -> bool {
        self.ctx.own_presence.status() == rekindle_types::presence::SessionStatus::Invisible
    }

    fn publish(&self, event: SubscriptionEvent) {
        self.ctx.publish_event(event);
    }

    async fn audit(&self, kind: rekindle_audit::AuditKind, detail: serde_json::Value) {
        let (Ok(owner), Ok(db)) = (self.ctx.require_owner(), self.ctx.db.current()) else {
            return;
        };
        crate::host::audit::append(
            &self.ctx.audit_chain,
            &db,
            &self.ctx.vault,
            &owner,
            kind,
            detail,
            self.ctx.clock.now_ms_i64(),
        );
    }

    async fn watch_friend(&self, peer: &str, profile_dht_key: &str) {
        match rekindle_presence::watch_friend(Arc::new(self.clone()), peer, profile_dht_key).await {
            // The name read is a GET: off the path that set the watch.
            Ok(Some(name_read)) => self.scope().spawn_or_drop("friend name read", name_read),
            Ok(None) => {}
            Err(e) => tracing::trace!(peer = %peer, error = %e, "friend presence not watched"),
        }
        // The profile is held only once its record opened; a failed open is
        // retried on the maintenance tick (`retry_failed_watches`).
        let lease = self.ctx.friend_watches.leases.lock().get(peer).copied();
        let pool = self.node().ok().and_then(|node| node.records());
        let held = lease.zip(pool).and_then(|(lease, pool)| pool.key_of(lease));
        let mut retries = self.ctx.friend_stores.watch_retries.lock();
        if held.as_deref() == Some(profile_dht_key) {
            retries.settled(profile_dht_key);
        } else {
            let target = WatchTarget::Profile {
                peer: peer.to_string(),
            };
            retries.failed(
                profile_dht_key,
                target,
                self.ctx.clock.now_ms(),
                self.ctx.rng.as_ref(),
            );
        }
    }

    async fn watch_participant(&self, peer: &str, record_key: &str) {
        watch::watch_participant(self, peer, record_key).await;
    }

    async fn unwatch_friend(&self, peer: &str) {
        // Releasing the friend's lease closes the record, which is what
        // ends its watch; the key-to-friend mapping is the projection's.
        watch::release_participant(self, peer).await;
        self.ctx.friend_stores.watch_retries.lock().forget(peer);
        let lease = self.ctx.friend_watches.leases.lock().remove(peer);
        let pool = self.node().ok().and_then(|node| node.records());
        if let (Some(lease), Some(pool)) = (lease, pool) {
            pool.release(lease).await;
        }
    }

    async fn keep_request_inbox(
        &self,
        record_key: &str,
        owner_keypair: &str,
        lease: LeaseId,
    ) -> Result<(), FriendError> {
        let kept = {
            let guard = self.ctx.vault.lock();
            guard
                .as_ref()
                .ok_or_else(|| FriendError::NotReady("the vault is locked".to_string()))
                .and_then(|vault| {
                    let entry = rekindle_vault::typed::owner_keys::OwnerEntry {
                        record_key: record_key.to_string(),
                        keypair: owner_keypair.to_string(),
                    };
                    rekindle_vault::typed::owner_keys::persist_inbox_owner(vault, &entry)
                        .map_err(|e| FriendError::Failed(format!("inbox owner not saved: {e}")))
                })
        };
        let pool = self
            .node()?
            .require_records()
            .map_err(|e| FriendError::NotReady(format!("records: {e}")))?;
        if let Err(e) = kept {
            pool.release(lease).await;
            return Err(e);
        }
        inbox_watch::hold(self, &pool, record_key, lease).await;
        Ok(())
    }

    async fn drop_request_inbox(&self, record_key: &str) {
        inbox_watch::release(self, record_key).await;
        if let Some(vault) = self.ctx.vault.lock().as_ref() {
            if let Err(e) = rekindle_vault::typed::owner_keys::forget_inbox_owner(vault, record_key)
            {
                tracing::warn!(error = %e, "invite inbox owner keypair not forgotten");
            }
        }
    }

    async fn reseal_own_presence(&self) {
        self.ctx.own_presence.reseal().await;
    }

    async fn rotate_own_profile(&self) -> Result<String, FriendError> {
        crate::host::own_records::rotate_profile(&self.ctx)
            .await
            .map_err(FriendError::Failed)
    }

    async fn hold_backend_intent(&self, held: BackendHold) -> Result<(), FriendError> {
        let (action, cancel, kind) = match held {
            BackendHold::SessionReset { peer } => (
                IpcRequest::SessionResetAccept {
                    public_key: peer.clone(),
                },
                Some(IpcRequest::SessionResetDecline { public_key: peer }),
                IntentKind::AcceptSessionReset,
            ),
            // Nothing was asked of the friend: Cancel only drops the hold
            // (ruling Q1).
            BackendHold::SessionReinitiate { peer } => (
                IpcRequest::SessionReinitiate { public_key: peer },
                None,
                IntentKind::ReinitiateSession,
            ),
            // The review is on the friend as we know them; the new key is
            // in the pending rotation it shows (owner ruling R1).
            BackendHold::IdentityRotation { peer, new_identity } => {
                tracing::debug!(peer = %peer, new_identity = %new_identity, "identity rotation held");
                (
                    IpcRequest::IdentityRotationAccept {
                        public_key: peer.clone(),
                    },
                    Some(IpcRequest::IdentityRotationDecline { public_key: peer }),
                    IntentKind::AcceptIdentityRotation,
                )
            }
        };
        let owner = IntentOwner {
            conn: None,
            identity: self.ctx.require_owner().map_err(|_| FriendError::Locked)?,
        };
        let selection = crate::host::intent_select::select(&self.ctx, &action, kind)
            .await
            .map_err(|e| FriendError::Failed(format!("{e:?}")))?;
        // One hold per friend and kind: a newer one replaces the older,
        // which goes without its cancel, but only once the newer fits: a
        // full gate refuses it and keeps the older (plan audit D-4).
        let now = self.ctx.clock.now_ms();
        if !self
            .ctx
            .intents
            .has_room(&owner, |held_action| same_hold(held_action, &action), now)
        {
            return Err(FriendError::Conflict(
                rekindle_idempotency::intent_gate::GateError::TooManyPending.to_string(),
            ));
        }
        self.ctx.intents.retain_held(|held_by, held_action| {
            !(held_by.conn.is_none()
                && held_by.identity == owner.identity
                && same_hold(held_action, &action))
        });
        let review = self
            .ctx
            .intents
            .hold(
                owner,
                selection,
                action,
                cancel,
                rekindle_idempotency::intent_gate::BACKEND_HOLD_TTL_MS,
                now,
            )
            .map_err(|e| FriendError::Conflict(e.to_string()))?;
        self.ctx
            .publish_event(SubscriptionEvent::IntentPending(review));
        Ok(())
    }

    fn own_prekey_lease(&self) -> Option<LeaseId> {
        *self.ctx.own_prekey_lease.lock()
    }

    fn scope(&self) -> Arc<rekindle_lifecycle::SessionScope> {
        self.ctx.unlock_scope_or_closed()
    }

    fn clock(&self) -> &dyn rekindle_utils::Clock {
        self.ctx.clock.as_ref()
    }

    fn rng(&self) -> &dyn rekindle_utils::RandomSource {
        self.ctx.rng.as_ref()
    }
}

/// Drop the backend hold on `peer`'s reset request: a direct
/// `SessionResetAccept` or `SessionResetDecline` answered it, so the held
/// review would act on a request that is gone (plan audit D-4).
pub(crate) fn release_reset_hold(ctx: &DaemonContext, peer: &str) {
    let Ok(identity) = ctx.require_owner() else {
        return;
    };
    let action = IpcRequest::SessionResetAccept {
        public_key: peer.to_string(),
    };
    ctx.intents.retain_held(|held_by, held_action| {
        !(held_by.conn.is_none() && held_by.identity == identity && same_hold(held_action, &action))
    });
}

/// Whether `a` and `b` are the same backend hold: the same kind for the
/// same friend.
fn same_hold(a: &IpcRequest, b: &IpcRequest) -> bool {
    match (a, b) {
        (
            IpcRequest::SessionResetAccept { public_key: x },
            IpcRequest::SessionResetAccept { public_key: y },
        )
        | (
            IpcRequest::IdentityRotationAccept { public_key: x },
            IpcRequest::IdentityRotationAccept { public_key: y },
        )
        | (
            IpcRequest::SessionReinitiate { public_key: x },
            IpcRequest::SessionReinitiate { public_key: y },
        ) => x == y,
        _ => false,
    }
}
