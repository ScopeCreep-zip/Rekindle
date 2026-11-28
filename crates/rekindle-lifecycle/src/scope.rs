//! `SessionScope`: every task that belongs to a session (a desktop login,
//! a daemon unlock, a call) is spawned through one, so ending the session
//! ends all of them (plan C4, D7).
//!
//! Built on tokio's graceful-shutdown pair: a `CancellationToken` tells
//! tasks to stop and a `TaskTracker` waits for them. The tracker alone
//! would keep accepting spawns after shutdown starts (`close` "does not
//! prevent you from spawning new tasks"), so the scope refuses them.
//!
//! Stopping is cooperative. A task is never dropped at cancellation: the
//! session's work is Veilid calls, which have no cancellation and log a
//! dropped future as an error, so a task stops at its own next safe point
//! (before a call, during a sleep, between ticks) and one-shot work runs
//! to completion within Veilid's own timeouts. Only a task still running
//! at the shutdown deadline is aborted (`evidence/c4-live-findings.md`).
//!
//! A panicking task is logged by name and reported to the scope's owner
//! once (`on_panic`). Shared session state may be half-mutated after a
//! panic, so the owner ends the session rather than carrying on
//! (`evidence/c4-session-scope-research.md`).

use std::collections::HashMap;
use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use futures_util::FutureExt as _;
use parking_lot::Mutex;
use tokio::task::AbortHandle;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;
use tracing::Instrument as _;

/// How long aborted tasks get to unwind before shutdown returns.
const ABORT_GRACE: Duration = Duration::from_secs(1);

/// The owner's reaction to a panicking task, given the task's name.
pub type OnPanic = Arc<dyn Fn(&'static str) + Send + Sync>;

/// A running task: its name, and its abort handle once spawned.
type Live = Arc<Mutex<HashMap<u64, (&'static str, Option<AbortHandle>)>>>;
type BoxedTask = Pin<Box<dyn Future<Output = ()> + Send>>;

/// The scope was shut down; the task was not started.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("session scope is closed; `{0}` was not started")]
pub struct ScopeClosed(pub &'static str);

/// Tasks still running when the shutdown deadline passed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("tasks aborted at the shutdown deadline: {0:?}")]
pub struct StuckTasks(pub Vec<&'static str>);

/// A task's place in every scope that tracks it, removed on drop.
struct Entry {
    lives: Vec<Live>,
    id: u64,
}

impl Drop for Entry {
    fn drop(&mut self) {
        for live in &self.lives {
            live.lock().remove(&self.id);
        }
    }
}

/// What a whole family of scopes (a root and its children) shares.
struct Family {
    on_panic: OnPanic,
    panicked: AtomicBool,
    next_id: AtomicU64,
}

/// A set of tasks that live and die together.
pub struct SessionScope {
    label: &'static str,
    cancel: CancellationToken,
    /// This scope's tracker first, then each ancestor's: a parent's
    /// shutdown waits for its children's tasks too.
    trackers: Vec<TaskTracker>,
    /// Running task names, this scope's first, then each ancestor's.
    lives: Vec<Live>,
    family: Arc<Family>,
}

impl std::fmt::Debug for SessionScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionScope")
            .field("label", &self.label)
            .field("closed", &self.is_closed())
            .field("tasks", &self.len())
            .finish_non_exhaustive()
    }
}

impl SessionScope {
    /// A root scope. `label` names it in logs ("login", "unlock").
    #[must_use]
    pub fn new(label: &'static str, on_panic: OnPanic) -> Arc<Self> {
        Arc::new(Self {
            label,
            cancel: CancellationToken::new(),
            trackers: vec![TaskTracker::new()],
            lives: vec![Live::default()],
            family: Arc::new(Family {
                on_panic,
                panicked: AtomicBool::new(false),
                next_id: AtomicU64::new(0),
            }),
        })
    }

    /// A scope that is already shut down: everything spawned on it is
    /// dropped. Stands in for a session that has ended (or not begun).
    #[must_use]
    pub fn closed(label: &'static str) -> Arc<Self> {
        let scope = Self::new(label, Arc::new(|_| {}));
        scope.cancel.cancel();
        scope.trackers[0].close();
        scope
    }

    /// A child scope (a call within a login): cancelled with its parent,
    /// shut down on its own, its tasks also waited for by the parent.
    #[must_use]
    pub fn child(self: &Arc<Self>, label: &'static str) -> Arc<Self> {
        let mut trackers = vec![TaskTracker::new()];
        trackers.extend(self.trackers.iter().cloned());
        let mut lives = vec![Live::default()];
        lives.extend(self.lives.iter().cloned());
        Arc::new(Self {
            label,
            cancel: self.cancel.child_token(),
            trackers,
            lives,
            family: Arc::clone(&self.family),
        })
    }

    /// Run one-shot work (a send, a fetch, a write) as a task of this
    /// scope. It runs to completion, and the scope's shutdown waits for it;
    /// work that loops, sleeps or makes several calls in sequence uses
    /// [`Self::spawn_with_token`] instead, so it can stop early.
    ///
    /// # Errors
    /// [`ScopeClosed`] once the scope is cancelled.
    pub fn spawn<F>(self: &Arc<Self>, name: &'static str, fut: F) -> Result<(), ScopeClosed>
    where
        F: Future<Output = ()> + Send + 'static,
    {
        self.track(name, fut)
    }

    /// Run a task that watches the scope's token itself and stops at its
    /// next safe point: before a Veilid call, during a sleep, between
    /// ticks. It may run its own shutdown procedure first, as tokio's
    /// graceful-shutdown guide describes. The shutdown deadline still
    /// bounds it.
    ///
    /// # Errors
    /// [`ScopeClosed`] once the scope is cancelled.
    pub fn spawn_with_token<F, Fut>(
        self: &Arc<Self>,
        name: &'static str,
        task: F,
    ) -> Result<(), ScopeClosed>
    where
        F: FnOnce(CancellationToken) -> Fut,
        Fut: Future<Output = ()> + Send + 'static,
    {
        if self.cancel.is_cancelled() {
            return Err(ScopeClosed(name));
        }
        let fut = task(self.cancel.clone());
        self.track(name, fut)
    }

    /// Track `fut` under `name` in this scope and every ancestor, report
    /// a panic to the owner, and spawn it in the spawner's current span, so
    /// its events (and its panic report) keep the context they were
    /// started in, as `tracing`'s `Instrument` documents for spawned tasks.
    fn track<F>(self: &Arc<Self>, name: &'static str, fut: F) -> Result<(), ScopeClosed>
    where
        F: Future<Output = ()> + Send + 'static,
    {
        if self.cancel.is_cancelled() {
            return Err(ScopeClosed(name));
        }
        let id = self.family.next_id.fetch_add(1, Ordering::Relaxed);
        for live in &self.lives {
            live.lock().insert(id, (name, None));
        }
        let entry = Entry {
            lives: self.lives.clone(),
            id,
        };
        let family = Arc::clone(&self.family);
        let task = async move {
            // Dropped when the task ends however it ends — finished,
            // panicked, or aborted at the deadline — so every scope's
            // count stays true.
            let _entry = entry;
            let outcome = AssertUnwindSafe(fut).catch_unwind().await;
            if outcome.is_err() {
                tracing::error!(task = name, "session task panicked");
                if !family.panicked.swap(true, Ordering::AcqRel) {
                    (family.on_panic)(name);
                }
            }
        }
        .instrument(tracing::Span::current());
        let mut boxed: BoxedTask = Box::pin(task);
        for ancestor in self.trackers.iter().skip(1) {
            boxed = Box::pin(ancestor.track_future(boxed));
        }
        let handle = self.trackers[0].spawn(boxed);
        // Recorded after the spawn; a task that already finished removed
        // its entry and needs no handle.
        for live in &self.lives {
            if let Some(entry) = live.lock().get_mut(&id) {
                entry.1 = Some(handle.abort_handle());
            }
        }
        Ok(())
    }

    /// [`Self::spawn`], dropping the work if the scope is already closed:
    /// it belonged to a session that is ending.
    pub fn spawn_or_drop<F>(self: &Arc<Self>, name: &'static str, fut: F)
    where
        F: Future<Output = ()> + Send + 'static,
    {
        if let Err(closed) = self.spawn(name, fut) {
            tracing::debug!(scope = self.label, error = %closed, "work dropped");
        }
    }

    /// [`Self::spawn_with_token`], dropping the work if the scope is
    /// already closed: it belonged to a session that is ending.
    pub fn spawn_with_token_or_drop<F, Fut>(self: &Arc<Self>, name: &'static str, task: F)
    where
        F: FnOnce(CancellationToken) -> Fut,
        Fut: Future<Output = ()> + Send + 'static,
    {
        if let Err(closed) = self.spawn_with_token(name, task) {
            tracing::debug!(scope = self.label, error = %closed, "work dropped");
        }
    }

    /// The scope's cancellation token, for code that selects on it.
    #[must_use]
    pub fn token(&self) -> CancellationToken {
        self.cancel.clone()
    }

    /// Tasks of this scope (and its children) still running.
    #[must_use]
    pub fn len(&self) -> usize {
        self.lives[0].lock().len()
    }

    /// The running tasks of this scope, by name with a count each, most
    /// numerous first: what a teardown is still waiting on.
    #[must_use]
    pub fn running(&self) -> Vec<(&'static str, usize)> {
        let mut counts: HashMap<&'static str, usize> = HashMap::new();
        for (name, _) in self.lives[0].lock().values() {
            *counts.entry(name).or_default() += 1;
        }
        let mut running: Vec<_> = counts.into_iter().collect();
        running.sort_unstable_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
        running
    }

    /// Whether no task of this scope is running.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Whether the scope has been shut down (or its parent has).
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.cancel.is_cancelled()
    }

    /// Cancel every task, refuse new ones, and wait for all of them however
    /// long they take: no task is ever aborted. For a scope whose tasks must
    /// run to completion — a record pool's Veilid calls, which have no
    /// cancellation, and whose abort mid-commit wedges Veilid's record
    /// store (plan C7.6j) — waited for off the user's path.
    pub async fn close_and_wait(&self) {
        self.cancel.cancel();
        self.trackers[0].close();
        self.trackers[0].wait().await;
        tracing::debug!(scope = self.label, "scope finished");
    }

    /// Cancel every task, refuse new ones, and wait up to `grace` for them
    /// to stop, aborting none: whether they all did. A logout gives the
    /// unlock's stages this before it drains their record pool, so a call
    /// in flight lands and its task stops at the token instead of failing
    /// on a pool that refuses it (plan VU4, live R1-2).
    pub async fn stop_within(&self, grace: Duration) -> bool {
        self.cancel.cancel();
        self.trackers[0].close();
        tokio::time::timeout(grace, self.trackers[0].wait())
            .await
            .is_ok()
    }

    /// Cancel every task, refuse new ones, and wait up to `deadline` for
    /// them to stop. A task still running then is aborted, so no task of
    /// the scope outlives this call.
    ///
    /// # Errors
    /// [`StuckTasks`] naming the tasks that had to be aborted.
    pub async fn shutdown(&self, deadline: Duration) -> Result<(), StuckTasks> {
        self.cancel.cancel();
        self.trackers[0].close();
        if tokio::time::timeout(deadline, self.trackers[0].wait())
            .await
            .is_ok()
        {
            tracing::info!(scope = self.label, remaining = 0, "scope shut down");
            return Ok(());
        }
        let overdue: Vec<(&'static str, Option<AbortHandle>)> =
            self.lives[0].lock().values().cloned().collect();
        for (_, handle) in &overdue {
            if let Some(handle) = handle {
                handle.abort();
            }
        }
        // An aborted task is dropped at its next poll, which removes its
        // entry; the tracker counts it then.
        let _ = tokio::time::timeout(ABORT_GRACE, self.trackers[0].wait()).await;
        let mut stuck: Vec<&'static str> = overdue.into_iter().map(|(name, _)| name).collect();
        stuck.sort_unstable();
        tracing::warn!(
            scope = self.label,
            aborted = stuck.len(),
            ?stuck,
            "scope shut down: tasks past the deadline were aborted"
        );
        Err(StuckTasks(stuck))
    }
}

#[cfg(test)]
#[path = "scope_tests.rs"]
mod tests;
