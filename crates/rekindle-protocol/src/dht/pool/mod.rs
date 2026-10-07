//! `RecordPool`: the only code that opens, closes, watches, reads or writes
//! a DHT record (plan C7.2, D4). One per logged-in session.
//!
//! veilid-core keeps one open entry per record per node: a re-open replaces
//! the writer **and** the safety selection, a close is node-wide and cancels
//! the watch (`storage_manager/open_record.rs:155-169`, `close_record.rs`).
//! The pool is the refcount Veilid does not keep
//! ([`rekindle_records::lease::LeaseTable`]), and it owns the Veilid
//! semantics that are easy to misread (`evidence/c7-pool-research.md` §2–§3):
//!
//! - **Anonymity.** Its routing context is built here, Safe with
//!   `hop_count ≥ ANONYMITY_HOP_FLOOR`, and every record goes through it, so
//!   no record is ever re-opened at Veilid's 1-hop default (V19).
//! - **Honest writes.** `set` passes `allow_offline: false` and reports a
//!   [`SetOutcome`]. veilid returns `Ok(None)` both for a write that landed
//!   and for one below consensus that stored nothing (`set_value.rs:170`);
//!   the local sequence number tells them apart (VeilidChat
//!   `dht_record.dart` `_localSubkeySeq`). Writes are not retried here: the
//!   outbox owns durability (D22).
//! - **One retry layer.** Opens, reads and inspects retry `TryAgain`,
//!   `KeyNotFound` and `TransactionNotFound`, waiting for
//!   `public_internet_ready`, within a 30 s budget (VeilidChat
//!   `DHTRetryStrategy`). Nothing above the pool retries.
//! - **One watch-death owner.** [`RecordPool::on_value_change`] re-arms a
//!   dead watch while a lease still watches it, with backoff and
//!   forgiveness. The return value of `watch_dht_values` is never read
//!   (it is not a grant, V20).
//! - **Calls are never dropped.** Every Veilid call runs as a task on the
//!   pool's scope; a caller that gives up drops only its wait, never an
//!   in-flight Veilid future (no cancellation, veilid #516; plan C4.L1).
//! - **The pool owns logout's waits** (plan C7.6g). [`RecordPool::drain`]
//!   releases every caller waiting on a call with
//!   [`ProtocolError::PoolClosed`] and refuses new calls; the calls run on.
//!   An open or create whose caller already left closes its own record, so
//!   no record stays open in Veilid without a lease. After the session's
//!   tasks stop, [`RecordPool::admit_teardown`] admits the teardown's own
//!   writes. [`RecordPool::end`] hands the records to the node's
//!   [`RecordCloser`], which closes them once the pool's calls finished,
//!   off the user's path; no call is ever aborted (C7.6i, C7.6j).

mod call;
mod closer;
mod durable;
mod io;
pub mod keepalive;
mod read;
mod safety;
mod watch;

/// Bound on compare-and-swap rounds for a read-merge-write whose write keeps
/// coming back `Superseded`: the channel append and the friend-inbox append.
/// Veilid stores the newer value locally on each supersede
/// (`set_value.rs:620-645`), so every round writes above what it lost to.
pub const CAS_ROUNDS: usize = 3;

pub use closer::RecordCloser;
pub use durable::UnsentWrite;
pub use io::SetOutcome;
pub use safety::safety_selection;
pub use watch::ValueChangeReport;

use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use futures::future::BoxFuture;
use parking_lot::Mutex;
use rekindle_lifecycle::SessionScope;
use rekindle_records::lease::{
    max_subkey_bytes, LeaseId, LeaseTable, OpenPlan, ReleasePlan, SchemaShape, SubkeySet,
};
use rekindle_types::config::SafetyProfile;
use tokio_util::sync::CancellationToken;
use veilid_core::{
    DHTRecordDescriptor, DHTSchema, KeyPair, RecordKey, RoutingContext, ValueSubkeyRangeSet,
    VeilidAPIError, CRYPTO_KIND_VLD0,
};

use crate::ProtocolError;

/// How long a read, open or inspect keeps retrying before it reports the
/// last transient error (VeilidChat `kDefaultDHTRetryTimeout`).
pub const RETRY_BUDGET: Duration = Duration::from_secs(30);
/// Wait between retries while the node already reports readiness.
const RETRY_DELAY: Duration = Duration::from_secs(1);
#[derive(Debug, Clone, Copy)]
pub(super) struct Opened {
    pub(super) key_cap: usize,
    /// The record's subkey count (from its schema), for a whole-record
    /// watch.
    pub(super) subkey_count: u32,
}

/// Which calls the pool admits (plan C7.6g).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Admit {
    /// The session is running.
    Open,
    /// Logout began: waits are released and new calls refused.
    Draining,
    /// The session's tasks stopped: the teardown's writes and closes run.
    Teardown,
}

/// What a call's task does with a result its caller no longer waits for.
type Unclaimed<T> = call::Unclaimed<RoutingContext, T>;

/// An open or create nobody claimed: close it, so the record does not stay
/// open in Veilid (writer key, watches) without a lease.
fn close_unclaimed(rc: RoutingContext, descriptor: DHTRecordDescriptor) -> BoxFuture<'static, ()> {
    Box::pin(async move {
        let key = descriptor.key();
        if let Err(e) = rc.close_dht_record(key.clone()).await {
            tracing::debug!(record = %key, error = %e, "closing an unclaimed record failed");
        }
    })
}

/// The session's DHT records.
pub struct RecordPool {
    rc: RoutingContext,
    scope: Arc<SessionScope>,
    ready: tokio::sync::watch::Receiver<bool>,
    table: Mutex<LeaseTable<KeyPair>>,
    records: Mutex<HashMap<String, (RecordKey, Opened)>>,
    key_locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    rearm: Mutex<HashMap<String, watch::Rearm>>,
    /// Durable writes that missed, held for re-push (`durable.rs`).
    held: Mutex<durable::HeldWrites>,
    held_changed: tokio::sync::Notify,
    /// Every durable write that settled (`durable.rs`, plan C7.13).
    settled: tokio::sync::broadcast::Sender<durable::Settled>,
    repush_started: std::sync::atomic::AtomicBool,
    admit: Mutex<Admit>,
    /// Cancelled by [`drain`](Self::drain): every wait then returns.
    waits: Mutex<CancellationToken>,
    /// Stops the re-push task at drain.
    repush_stop: CancellationToken,
    /// The node's closer: ends this pool's records, and holds a re-open of
    /// a record an ended session is still closing.
    closer: Arc<RecordCloser>,
    /// This pool, for the tasks it starts on its own scope.
    this: std::sync::Weak<RecordPool>,
}

impl std::fmt::Debug for RecordPool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RecordPool")
            .field("held", &self.table.lock().keys().count())
            .finish_non_exhaustive()
    }
}

pub(super) fn transient(e: &VeilidAPIError) -> bool {
    matches!(
        e,
        VeilidAPIError::TryAgain { .. }
            | VeilidAPIError::KeyNotFound { .. }
            | VeilidAPIError::TransactionNotFound { .. }
    )
}

pub(super) fn to_protocol(e: &VeilidAPIError) -> ProtocolError {
    if matches!(e, VeilidAPIError::Shutdown) {
        ProtocolError::PoolClosed
    } else if transient(e) {
        ProtocolError::DhtRecordUnreachable(e.to_string())
    } else {
        ProtocolError::DhtError(e.to_string())
    }
}

pub(super) fn range(subkeys: &SubkeySet) -> ValueSubkeyRangeSet {
    subkeys.iter().copied().collect()
}

impl RecordPool {
    /// A pool over `rc`'s Veilid API, with the safety selection pinned from
    /// `profile`. `scope` runs the pool's calls (ended by [`end`](Self::end),
    /// never aborted); `ready` follows `public_internet_ready`; `closer` is
    /// the node's.
    ///
    /// # Errors
    /// The routing context cannot take the safety selection.
    pub fn new(
        rc: &RoutingContext,
        profile: &SafetyProfile,
        scope: Arc<SessionScope>,
        ready: tokio::sync::watch::Receiver<bool>,
        closer: Arc<RecordCloser>,
    ) -> Result<Arc<Self>, ProtocolError> {
        let rc = rc
            .clone()
            .with_safety(safety_selection(profile))
            .map_err(|e| ProtocolError::RoutingError(format!("pool safety selection: {e}")))?;
        Ok(Arc::new_cyclic(|this| Self {
            this: this.clone(),
            rc,
            scope,
            ready,
            table: Mutex::new(LeaseTable::default()),
            records: Mutex::new(HashMap::new()),
            key_locks: Mutex::new(HashMap::new()),
            rearm: Mutex::new(HashMap::new()),
            held: Mutex::new(HashMap::new()),
            held_changed: tokio::sync::Notify::new(),
            settled: tokio::sync::broadcast::channel(256).0,
            repush_started: std::sync::atomic::AtomicBool::new(false),
            admit: Mutex::new(Admit::Open),
            waits: Mutex::new(CancellationToken::new()),
            repush_stop: CancellationToken::new(),
            closer,
        }))
    }

    /// The scope the pool's Veilid calls run on.
    pub fn scope(&self) -> &Arc<SessionScope> {
        &self.scope
    }

    /// The routing context every record goes through.
    pub fn routing_context(&self) -> &RoutingContext {
        &self.rc
    }

    /// Begin logout (plan C7.6g): every caller waiting on a call returns
    /// [`ProtocolError::PoolClosed`] now, new calls are refused, and the
    /// re-push task stops. The calls in flight run on to completion on the
    /// pool's scope. Held writes stay held ([`held_writes`](Self::held_writes)).
    pub fn drain(&self) {
        *self.admit.lock() = Admit::Draining;
        self.waits.lock().cancel();
        self.repush_stop.cancel();
    }

    /// Admit the teardown's own calls once the session's tasks stopped (the
    /// last status write). The re-push task stays stopped.
    pub fn admit_teardown(&self) {
        let mut admit = self.admit.lock();
        if *admit != Admit::Teardown {
            *admit = Admit::Teardown;
            *self.waits.lock() = CancellationToken::new();
        }
    }

    /// Run one Veilid call as a task on the pool's scope and wait for it.
    /// Giving up on the wait never drops the call.
    pub(super) async fn run<T, F, Fut>(
        &self,
        name: &'static str,
        call: F,
    ) -> Result<T, VeilidAPIError>
    where
        T: Send + 'static,
        F: FnOnce(RoutingContext) -> Fut,
        Fut: Future<Output = Result<T, VeilidAPIError>> + Send + 'static,
    {
        self.run_with(name, call, None).await
    }

    /// [`run`](Self::run), with what the task does with a result whose
    /// caller left ([`call::handover`]).
    async fn run_with<T, F, Fut>(
        &self,
        name: &'static str,
        call: F,
        unclaimed: Option<Unclaimed<T>>,
    ) -> Result<T, VeilidAPIError>
    where
        T: Send + 'static,
        F: FnOnce(RoutingContext) -> Fut,
        Fut: Future<Output = Result<T, VeilidAPIError>> + Send + 'static,
    {
        if *self.admit.lock() == Admit::Draining {
            return Err(VeilidAPIError::Shutdown);
        }
        let waits = self.waits.lock().clone();
        let rc = self.rc.clone();
        let fut = call(rc.clone());
        call::handover(
            &self.scope,
            name,
            rc,
            fut,
            unclaimed,
            &waits,
            VeilidAPIError::Shutdown,
        )
        .await
    }

    /// [`run`](Self::run) inside the one retry layer: transient errors
    /// retry after readiness (or a 1 s pause when already ready), until
    /// [`RETRY_BUDGET`].
    pub(super) async fn run_retrying<T, F, Fut>(
        &self,
        name: &'static str,
        call: F,
    ) -> Result<T, VeilidAPIError>
    where
        T: Send + 'static,
        F: Fn(RoutingContext) -> Fut,
        Fut: Future<Output = Result<T, VeilidAPIError>> + Send + 'static,
    {
        self.run_retrying_with(name, call, None).await
    }

    /// [`run_retrying`](Self::run_retrying) with an [`Unclaimed`] handler
    /// for each attempt.
    async fn run_retrying_with<T, F, Fut>(
        &self,
        name: &'static str,
        call: F,
        unclaimed: Option<Unclaimed<T>>,
    ) -> Result<T, VeilidAPIError>
    where
        T: Send + 'static,
        F: Fn(RoutingContext) -> Fut,
        Fut: Future<Output = Result<T, VeilidAPIError>> + Send + 'static,
    {
        let deadline = tokio::time::Instant::now() + RETRY_BUDGET;
        let mut ready = self.ready.clone();
        loop {
            match self.run_with(name, &call, unclaimed).await {
                Err(e) if transient(&e) && tokio::time::Instant::now() < deadline => {
                    tracing::debug!(name, error = %e, "transient DHT failure; retrying");
                    let wait = async {
                        if *ready.borrow() {
                            tokio::time::sleep(RETRY_DELAY).await;
                        } else {
                            let _ = ready.wait_for(|r| *r).await;
                        }
                    };
                    let waits = self.waits.lock().clone();
                    let stopped = tokio::select! {
                        () = waits.cancelled() => true,
                        _ = tokio::time::timeout_at(deadline, wait) => false,
                    };
                    if stopped {
                        return Err(VeilidAPIError::Shutdown);
                    }
                }
                other => return other,
            }
        }
    }

    pub(super) fn key_lock(&self, key: &str) -> Arc<tokio::sync::Mutex<()>> {
        self.key_locks
            .lock()
            .entry(key.to_string())
            .or_default()
            .clone()
    }

    pub(super) fn lookup(&self, id: LeaseId) -> Result<(RecordKey, Opened), ProtocolError> {
        let table = self.table.lock();
        let key = table.key(id).ok_or(ProtocolError::LeaseNotHeld(id.0))?;
        self.records
            .lock()
            .get(key)
            .cloned()
            .ok_or(ProtocolError::LeaseNotHeld(id.0))
    }

    fn remember(&self, key: &RecordKey, schema: &DHTSchema) {
        let subkey_count = u32::try_from(schema.subkey_count()).unwrap_or(u32::MAX);
        let cap = max_subkey_bytes(SchemaShape { subkey_count });
        self.records.lock().insert(
            key.to_string(),
            (
                key.clone(),
                Opened {
                    key_cap: cap,
                    subkey_count,
                },
            ),
        );
    }

    /// Borrow `key`, writable with `writer` when given. Opens are serialized
    /// per record, so two borrowers never race Veilid's record lock.
    ///
    /// # Errors
    /// The record cannot be opened within the retry budget.
    pub async fn acquire(
        &self,
        key: &RecordKey,
        writer: Option<KeyPair>,
    ) -> Result<LeaseId, ProtocolError> {
        let name = key.to_string();
        let lock = self.key_lock(&name);
        let _serial = lock.lock().await;
        let (id, plan) = self.table.lock().acquire(&name, writer);
        let open_with = match &plan {
            OpenPlan::AlreadyOpen => return Ok(id),
            OpenPlan::Open { writer } => writer.clone(),
            OpenPlan::Upgrade { writer } => Some(writer.clone()),
        };
        // An ended session may still be closing this record: opening it now
        // would be undone by that close.
        self.closer.wait_closed(&name).await;
        let k = key.clone();
        // A first open whose caller left closes itself; an upgrade re-opens
        // a record the table still holds, which shutdown closes.
        let unclaimed: Option<Unclaimed<DHTRecordDescriptor>> =
            matches!(plan, OpenPlan::Open { .. }).then_some(close_unclaimed);
        match self
            .run_retrying_with(
                "dht open",
                move |rc| {
                    let (k, w) = (k.clone(), open_with.clone());
                    async move { rc.open_dht_record(k, w).await }
                },
                unclaimed,
            )
            .await
        {
            Ok(descriptor) => {
                self.remember(key, &descriptor.schema());
                if self.table.lock().writer(&name).is_some() {
                    self.wake_held(&name);
                }
                Ok(id)
            }
            Err(e) => {
                self.table.lock().abort(id, &plan);
                Err(to_protocol(&e))
            }
        }
    }

    /// Create a record owned by `owner` (a fresh key when `None`) and hold
    /// it writable. Returns the lease and the owner keypair.
    ///
    /// # Errors
    /// Veilid refused the create, or returned no owner secret.
    pub async fn create(
        &self,
        schema: DHTSchema,
        owner: Option<KeyPair>,
    ) -> Result<(LeaseId, RecordKey, KeyPair), ProtocolError> {
        let descriptor = self
            .run_with(
                "dht create",
                move |rc| async move { rc.create_dht_record(CRYPTO_KIND_VLD0, schema, owner).await },
                Some(close_unclaimed),
            )
            .await
            .map_err(|e| to_protocol(&e))?;
        let key = descriptor.key();
        let secret = descriptor
            .owner_secret()
            .ok_or_else(|| ProtocolError::DhtError("created record has no owner secret".into()))?;
        let keypair = KeyPair::new_from_parts(descriptor.owner(), secret.value());
        let (id, _) = self
            .table
            .lock()
            .acquire(&key.to_string(), Some(keypair.clone()));
        self.remember(&key, &descriptor.schema());
        Ok((id, key, keypair))
    }

    /// The record key a lease borrows, while it is held.
    #[must_use]
    pub fn key_of(&self, id: LeaseId) -> Option<String> {
        self.table.lock().key(id).map(str::to_string)
    }

    /// End a borrow; the record closes when it was the last.
    pub async fn release(&self, id: LeaseId) {
        let Some(name) = self.table.lock().key(id).map(str::to_string) else {
            return;
        };
        let lock = self.key_lock(&name);
        let _serial = lock.lock().await;
        let plan = self.table.lock().release_plan(id);
        match plan {
            ReleasePlan::Kept => {}
            ReleasePlan::Closed(closed) => self.close(&closed).await,
            // The released borrow watched subkeys nobody else does.
            ReleasePlan::Rewatch(record, watch) => {
                let key = self.records.lock().get(&record).map(|(key, _)| key.clone());
                if let Some(key) = key {
                    if let Err(e) = self.apply_watch(&key, watch).await {
                        tracing::debug!(record, error = %e, "shrinking the watch failed");
                    }
                }
            }
        }
    }

    async fn close(&self, name: &str) {
        let Some((key, _)) = self.records.lock().remove(name) else {
            return;
        };
        self.rearm.lock().remove(name);
        if let Err(e) = self
            .run("dht close", move |rc| async move {
                rc.close_dht_record(key).await
            })
            .await
        {
            tracing::debug!(record = name, error = %e, "closing record failed");
        }
    }

    /// The largest value one subkey of the leased record accepts.
    ///
    /// # Errors
    /// The lease is not held.
    pub fn subkey_cap(&self, id: LeaseId) -> Result<usize, ProtocolError> {
        Ok(self.lookup(id)?.1.key_cap)
    }

    /// End the session's records (logout, after the teardown's writes).
    /// Leases end, new calls are refused, and the node's closer closes every
    /// record once the pool's calls in flight have finished; returns at once
    /// with how many records it handed over (plan C7.6i).
    pub fn end(&self) -> usize {
        *self.admit.lock() = Admit::Draining;
        self.repush_stop.cancel();
        self.table.lock().clear();
        self.rearm.lock().clear();
        let keys: Vec<RecordKey> = self
            .records
            .lock()
            .drain()
            .map(|(_, (key, _))| key)
            .collect();
        let count = keys.len();
        self.closer
            .close_after(Arc::clone(&self.scope), self.rc.clone(), keys);
        count
    }
}
