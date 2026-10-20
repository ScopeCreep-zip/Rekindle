//! The friend and 1:1 operations' Host (plan E2.1): one implementation of
//! `rekindle_transport::operations::friend::FriendHost` over the
//! [`DaemonContext`], used by both hosts. The friend presence adapter
//! (`presence`), the maintenance task (`maintenance`) and the pending
//! message retry (`pending_retry`) run over it too.

mod maintenance;
mod pending_retry;
mod presence;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use parking_lot::{Mutex, RwLock};
use rekindle_ipc::protocol::IpcRequest;
use rekindle_records::lease::LeaseId;
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

/// Each watched friend's profile lease, and the friends whose watch failed
/// (polled instead, Veilid GitLab #377).
#[derive(Default)]
pub struct FriendWatches {
    leases: Mutex<HashMap<String, LeaseId>>,
    unwatched: RwLock<HashSet<String>>,
}

impl FriendWatches {
    /// Mark `peers` unwatched, so the friend sync re-establishes their
    /// watches (a network reattach drops them).
    pub(crate) fn mark_unwatched(&self, peers: impl IntoIterator<Item = String>) {
        self.unwatched.write().extend(peers);
    }
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
        if let Err(e) =
            rekindle_presence::watch_friend(Arc::new(self.clone()), peer, profile_dht_key).await
        {
            tracing::trace!(peer = %peer, error = %e, "friend presence not watched");
        }
    }

    async fn unwatch_friend(&self, peer: &str) {
        // Releasing the friend's lease closes the record, which is what
        // ends its watch; the key-to-friend mapping is the projection's.
        let lease = self.ctx.friend_watches.leases.lock().remove(peer);
        self.ctx.friend_watches.unwatched.write().remove(peer);
        let pool = self.node().ok().and_then(|node| node.records());
        if let (Some(lease), Some(pool)) = (lease, pool) {
            pool.release(lease).await;
        }
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
                IpcRequest::SessionResetDecline { public_key: peer },
                IntentKind::AcceptSessionReset,
            ),
            // The review is on the friend as we know them; the new key is
            // in the pending rotation it shows (owner ruling R1).
            BackendHold::IdentityRotation { peer, new_identity } => {
                tracing::debug!(peer = %peer, new_identity = %new_identity, "identity rotation held");
                (
                    IpcRequest::IdentityRotationAccept {
                        public_key: peer.clone(),
                    },
                    IpcRequest::IdentityRotationDecline { public_key: peer },
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
        // which goes without its cancel.
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
                Some(cancel),
                rekindle_idempotency::intent_gate::BACKEND_HOLD_TTL_MS,
                self.ctx.clock.now_ms(),
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
        ) => x == y,
        _ => false,
    }
}

/// Watch every accepted friend's profile for their presence, in `scope`.
pub(crate) fn watch_friends(ctx: &DaemonContext, scope: &Arc<rekindle_lifecycle::SessionScope>) {
    let host = HostFriends::of(ctx);
    scope.spawn_or_drop("friend watches", async move {
        let friends: Vec<(String, String)> = host
            .ctx
            .session
            .read()
            .as_ref()
            .map(|s| {
                s.friends
                    .values()
                    .filter(|f| {
                        f.friendship_state == rekindle_types::friend::FriendshipState::Accepted
                    })
                    .filter_map(|f| f.dht_record_key.clone().map(|k| (f.public_key.clone(), k)))
                    .collect()
            })
            .unwrap_or_default();
        for (peer, profile_key) in friends {
            host.watch_friend(&peer, &profile_key).await;
        }
    });
}
