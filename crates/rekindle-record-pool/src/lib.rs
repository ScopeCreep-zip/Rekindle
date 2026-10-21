//! The record pool (ADR 0021): `RecordPool`, the only code that opens, closes, watches, reads or writes
//! a DHT record (plan C7.2, D4). One per logged-in session.
//!
//! veilid-core keeps one open entry per record per node: a re-open replaces
//! the writer **and** the safety selection, a close is node-wide and cancels
//! the watch (`storage_manager/open_record.rs:155-169`, `close_record.rs`).
//! The pool is the refcount Veilid does not keep
//! ([`rekindle_records::lease::LeaseTable`]), and it owns the Veilid
//! semantics that are easy to misread (`evidence/c7-pool-research.md` §2–§3):
//!
//! - **Anonymity.** Its backend's routing context is Safe with
//!   `hop_count ≥ ANONYMITY_HOP_FLOOR` (`rekindle_protocol::dht::backend::VeilidDht`), and
//!   every record goes through it, so no record is ever re-opened at
//!   Veilid's 1-hop default (V19).
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
//!   [`PortError::Closed`] and refuses new calls; the calls run on.
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
mod port;
mod read;
#[cfg(test)]
mod tests;
mod watch;

/// Bound on compare-and-swap rounds for a read-merge-write whose write keeps
/// coming back `Superseded`: the channel append and the friend-inbox append.
/// Veilid stores the newer value locally on each supersede
/// (`set_value.rs:620-645`), so every round writes above what it lost to.
pub const CAS_ROUNDS: usize = 3;

pub use closer::RecordCloser;
pub use io::set_failed;
pub use port::as_port;

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
use rekindle_records::port::PortError;
use rekindle_records::port::{
    DhtBackend, DhtCallError, DhtErrorKind, RecordDescriptor, RecordSchema, WriterKey,
};
use tokio_util::sync::CancellationToken;

/// How long a read, open or inspect keeps retrying before it reports the
/// last transient error (VeilidChat `kDefaultDHTRetryTimeout`).
pub const RETRY_BUDGET: Duration = Duration::from_secs(30);
/// Wait between retries while the node already reports readiness.
const RETRY_DELAY: Duration = Duration::from_secs(1);
#[derive(Debug, Clone, Copy)]
pub(crate) struct OpenRecord {
    pub(crate) key_cap: usize,
    /// The record's subkey count (from its schema), for a whole-record
    /// watch.
    pub(crate) subkey_count: u32,
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
type Unclaimed<T> = call::Unclaimed<Arc<dyn DhtBackend>, T>;

/// An open or create nobody claimed: close it, so the record does not stay
/// open in Veilid (writer key, watches) without a lease.
fn close_unclaimed(
    backend: Arc<dyn DhtBackend>,
    descriptor: RecordDescriptor,
) -> BoxFuture<'static, ()> {
    Box::pin(async move {
        let key = descriptor.key;
        if let Err(e) = backend.close(&key).await {
            tracing::debug!(record = %key, error = %e, "closing an unclaimed record failed");
        }
    })
}

/// The session's DHT records.
pub struct RecordPool {
    backend: Arc<dyn DhtBackend>,
    scope: Arc<SessionScope>,
    ready: tokio::sync::watch::Receiver<bool>,
    table: Mutex<LeaseTable<WriterKey>>,
    records: Mutex<HashMap<String, OpenRecord>>,
    key_locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    rearm: Mutex<HashMap<String, watch::Rearm>>,
    /// Durable writes that missed, held for re-push (`durable.rs`).
    held: Mutex<durable::HeldWrites>,
    held_changed: tokio::sync::Notify,
    /// Every durable write that settled (`durable.rs`, plan C7.13).
    settled: tokio::sync::broadcast::Sender<rekindle_records::port::Settled>,
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

pub(crate) fn transient(e: &DhtCallError) -> bool {
    matches!(
        e.kind,
        DhtErrorKind::TryAgain | DhtErrorKind::KeyNotFound | DhtErrorKind::TransactionNotFound
    )
}

/// How the pool reports a backend error to its callers: `Shutdown` is
/// [`PortError::Closed`], a transient kind is [`PortError::Unreachable`],
/// anything else [`PortError::Dht`].
#[must_use]
pub fn to_port(e: &DhtCallError) -> PortError {
    if e.kind == DhtErrorKind::Shutdown {
        PortError::Closed
    } else if transient(e) {
        PortError::Unreachable(e.to_string())
    } else {
        PortError::Dht(e.to_string())
    }
}

pub(crate) fn range(subkeys: &SubkeySet) -> Vec<u32> {
    subkeys.iter().copied().collect()
}

impl RecordPool {
    /// A pool over `backend`. `scope` runs the pool's calls (ended by
    /// [`end`](Self::end), never aborted); `ready` follows
    /// `public_internet_ready`; `closer` is the node's.
    #[must_use]
    pub fn new(
        backend: Arc<dyn DhtBackend>,
        scope: Arc<SessionScope>,
        ready: tokio::sync::watch::Receiver<bool>,
        closer: Arc<RecordCloser>,
    ) -> Arc<Self> {
        Arc::new_cyclic(|this| Self {
            this: this.clone(),
            backend,
            scope,
            ready,
            table: Mutex::new(LeaseTable::default()),
            records: Mutex::new(HashMap::new()),
            key_locks: Mutex::new(HashMap::new()),
            rearm: Mutex::new(HashMap::new()),
            held: Mutex::new(durable::HeldWrites::new()),
            held_changed: tokio::sync::Notify::new(),
            settled: tokio::sync::broadcast::channel(256).0,
            repush_started: std::sync::atomic::AtomicBool::new(false),
            admit: Mutex::new(Admit::Open),
            waits: Mutex::new(CancellationToken::new()),
            repush_stop: CancellationToken::new(),
            closer,
        })
    }

    /// The scope the pool's Veilid calls run on.
    pub fn scope(&self) -> &Arc<SessionScope> {
        &self.scope
    }

    /// Begin logout (plan C7.6g): every caller waiting on a call returns
    /// [`PortError::Closed`] now, new calls are refused, and the
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
    pub(crate) async fn run<T, F, Fut>(
        &self,
        name: &'static str,
        call: F,
    ) -> Result<T, DhtCallError>
    where
        T: Send + 'static,
        F: FnOnce(Arc<dyn DhtBackend>) -> Fut,
        Fut: Future<Output = Result<T, DhtCallError>> + Send + 'static,
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
    ) -> Result<T, DhtCallError>
    where
        T: Send + 'static,
        F: FnOnce(Arc<dyn DhtBackend>) -> Fut,
        Fut: Future<Output = Result<T, DhtCallError>> + Send + 'static,
    {
        if *self.admit.lock() == Admit::Draining {
            return Err(DhtCallError::shutdown());
        }
        let waits = self.waits.lock().clone();
        let backend = Arc::clone(&self.backend);
        let fut = call(Arc::clone(&backend));
        call::handover(
            &self.scope,
            name,
            backend,
            fut,
            unclaimed,
            &waits,
            DhtCallError::shutdown(),
        )
        .await
    }

    /// [`run`](Self::run) inside the one retry layer: transient errors
    /// retry after readiness (or a 1 s pause when already ready), until
    /// [`RETRY_BUDGET`].
    pub(crate) async fn run_retrying<T, F, Fut>(
        &self,
        name: &'static str,
        call: F,
    ) -> Result<T, DhtCallError>
    where
        T: Send + 'static,
        F: Fn(Arc<dyn DhtBackend>) -> Fut,
        Fut: Future<Output = Result<T, DhtCallError>> + Send + 'static,
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
    ) -> Result<T, DhtCallError>
    where
        T: Send + 'static,
        F: Fn(Arc<dyn DhtBackend>) -> Fut,
        Fut: Future<Output = Result<T, DhtCallError>> + Send + 'static,
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
                        return Err(DhtCallError::shutdown());
                    }
                }
                other => return other,
            }
        }
    }

    pub(crate) fn key_lock(&self, key: &str) -> Arc<tokio::sync::Mutex<()>> {
        self.key_locks
            .lock()
            .entry(key.to_string())
            .or_default()
            .clone()
    }

    pub(crate) fn lookup(&self, id: LeaseId) -> Result<(String, OpenRecord), PortError> {
        let table = self.table.lock();
        let key = table.key(id).ok_or(PortError::LeaseNotHeld(id.0))?;
        let opened = self
            .records
            .lock()
            .get(key)
            .copied()
            .ok_or(PortError::LeaseNotHeld(id.0))?;
        Ok((key.to_string(), opened))
    }

    fn remember(&self, key: &str, subkey_count: u32) {
        let cap = max_subkey_bytes(SchemaShape { subkey_count });
        self.records.lock().insert(
            key.to_string(),
            OpenRecord {
                key_cap: cap,
                subkey_count,
            },
        );
    }

    /// Borrow `key`, writable with `writer` when given. Opens are serialized
    /// per record, so two borrowers never race Veilid's record lock.
    ///
    /// # Errors
    /// The record cannot be opened within the retry budget.
    pub async fn acquire(
        &self,
        key: &str,
        writer: Option<WriterKey>,
    ) -> Result<LeaseId, PortError> {
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
        let k = key.to_string();
        // A first open whose caller left closes itself; an upgrade re-opens
        // a record the table still holds, which shutdown closes.
        let unclaimed: Option<Unclaimed<RecordDescriptor>> =
            matches!(plan, OpenPlan::Open { .. }).then_some(close_unclaimed);
        match self
            .run_retrying_with(
                "dht open",
                move |backend| {
                    let (k, w) = (k.clone(), open_with.clone());
                    async move { backend.open(&k, w.as_ref()).await }
                },
                unclaimed,
            )
            .await
        {
            Ok(descriptor) => {
                self.remember(key, descriptor.subkey_count);
                if self.table.lock().writer(&name).is_some() {
                    self.wake_held(&name);
                }
                Ok(id)
            }
            Err(e) => {
                self.table.lock().abort(id, &plan);
                Err(to_port(&e))
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
        schema: RecordSchema,
        owner: Option<WriterKey>,
    ) -> Result<(LeaseId, String, WriterKey), PortError> {
        let descriptor = self
            .run_with(
                "dht create",
                move |backend| async move { backend.create(&schema, owner.as_ref()).await },
                Some(close_unclaimed),
            )
            .await
            .map_err(|e| to_port(&e))?;
        let keypair = descriptor
            .owner
            .clone()
            .ok_or_else(|| PortError::Dht("created record has no owner secret".into()))?;
        let key = descriptor.key;
        let (id, _) = self.table.lock().acquire(&key, Some(keypair.clone()));
        self.remember(&key, descriptor.subkey_count);
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
                let open = self.records.lock().contains_key(&record);
                if open {
                    if let Err(e) = self.apply_watch(&record, watch).await {
                        tracing::debug!(record, error = %e, "shrinking the watch failed");
                    }
                }
            }
        }
    }

    async fn close(&self, name: &str) {
        if self.records.lock().remove(name).is_none() {
            return;
        }
        self.rearm.lock().remove(name);
        let key = name.to_string();
        if let Err(e) = self
            .run("dht close", move |backend| async move {
                backend.close(&key).await
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
    pub fn subkey_cap(&self, id: LeaseId) -> Result<usize, PortError> {
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
        let mut keys: Vec<String> = self.records.lock().drain().map(|(key, _)| key).collect();
        // One close order on every run.
        keys.sort_unstable();
        let count = keys.len();
        self.closer
            .close_after(Arc::clone(&self.scope), Arc::clone(&self.backend), keys);
        count
    }
}
