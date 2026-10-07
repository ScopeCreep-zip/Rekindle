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
    /// a panic to the owner, and spawn it.
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
        };
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
mod tests {
    use super::*;

    fn quiet() -> OnPanic {
        Arc::new(|_| {})
    }

    /// A cooperative task: runs until its scope is cancelled.
    fn until_stopped(scope: &Arc<SessionScope>, name: &'static str) {
        scope
            .spawn_with_token(name, |stop| async move { stop.cancelled().await })
            .unwrap();
    }

    #[tokio::test]
    async fn running_counts_tasks_by_name() {
        let scope = SessionScope::new("test", quiet());
        until_stopped(&scope, "b");
        until_stopped(&scope, "a");
        until_stopped(&scope, "a");
        tokio::task::yield_now().await;
        assert_eq!(scope.running(), vec![("a", 2), ("b", 1)]);
        scope.shutdown(Duration::from_secs(1)).await.unwrap();
        assert!(scope.running().is_empty());
    }

    #[tokio::test]
    async fn shutdown_ends_every_task_and_refuses_new_ones() {
        let scope = SessionScope::new("test", quiet());
        for _ in 0..3 {
            until_stopped(&scope, "loop");
        }
        assert_eq!(scope.len(), 3);
        scope.shutdown(Duration::from_secs(1)).await.unwrap();
        assert!(scope.is_empty());
        assert_eq!(scope.spawn("late", async {}), Err(ScopeClosed("late")));
    }

    /// `close_and_wait` refuses new work and waits for one-shot work to
    /// finish, however long it takes.
    #[tokio::test]
    async fn close_and_wait_lets_one_shot_work_finish_and_refuses_new_work() {
        let scope = SessionScope::new("test", quiet());
        let (done_tx, done_rx) = tokio::sync::oneshot::channel::<()>();
        let finished = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&finished);
        scope
            .spawn("call", async move {
                let _ = done_rx.await;
                flag.store(true, Ordering::SeqCst);
            })
            .unwrap();
        let release = async {
            tokio::time::sleep(Duration::from_millis(20)).await;
            assert!(
                !finished.load(Ordering::SeqCst),
                "still waiting on the call"
            );
            let _ = done_tx.send(());
        };
        tokio::join!(scope.close_and_wait(), release);
        assert!(finished.load(Ordering::SeqCst));
        assert_eq!(scope.spawn("late", async {}), Err(ScopeClosed("late")));
    }

    /// One-shot work is not cut off mid-flight: shutdown waits for it.
    #[tokio::test]
    async fn one_shot_work_runs_to_completion() {
        let scope = SessionScope::new("test", quiet());
        let done = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&done);
        scope
            .spawn("send", async move {
                tokio::time::sleep(Duration::from_millis(50)).await;
                flag.store(true, Ordering::Release);
            })
            .unwrap();
        scope.shutdown(Duration::from_secs(1)).await.unwrap();
        assert!(done.load(Ordering::Acquire));
    }

    /// A task that ignores its token is aborted at the deadline, named,
    /// and never runs again.
    #[tokio::test]
    async fn overdue_tasks_are_aborted_and_named() {
        let scope = SessionScope::new("test", quiet());
        let resumed = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&resumed);
        scope
            .spawn("stubborn", async move {
                tokio::time::sleep(Duration::from_millis(300)).await;
                flag.store(true, Ordering::Release);
            })
            .unwrap();
        assert_eq!(
            scope.shutdown(Duration::from_millis(50)).await,
            Err(StuckTasks(vec!["stubborn"]))
        );
        assert!(scope.is_empty());
        tokio::time::sleep(Duration::from_millis(400)).await;
        assert!(
            !resumed.load(Ordering::Acquire),
            "an aborted task must not resume"
        );
    }

    #[tokio::test]
    async fn a_panic_is_reported_once_by_name() {
        let reported = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&reported);
        let scope = SessionScope::new("test", Arc::new(move |name| sink.lock().push(name)));
        scope.spawn("first", async { panic!("bug") }).unwrap();
        scope.spawn("second", async { panic!("bug") }).unwrap();
        scope.shutdown(Duration::from_secs(1)).await.unwrap();
        let reported = reported.lock();
        assert_eq!(reported.len(), 1);
        assert!(matches!(reported[0], "first" | "second"));
    }

    #[tokio::test]
    async fn a_parent_waits_for_and_cancels_its_children() {
        let parent = SessionScope::new("login", quiet());
        let call = parent.child("call");
        until_stopped(&call, "media");
        assert_eq!(parent.len(), 1, "the parent counts its child's tasks");
        parent.shutdown(Duration::from_secs(1)).await.unwrap();
        assert!(call.is_closed());
        assert_eq!(call.spawn("late", async {}), Err(ScopeClosed("late")));
    }

    /// A parent past its deadline aborts its children's tasks too.
    #[tokio::test]
    async fn a_parent_aborts_overdue_child_tasks() {
        let parent = SessionScope::new("login", quiet());
        let call = parent.child("call");
        call.spawn("media", std::future::pending()).unwrap();
        assert_eq!(
            parent.shutdown(Duration::from_millis(50)).await,
            Err(StuckTasks(vec!["media"]))
        );
        assert!(call.is_empty());
    }

    #[tokio::test]
    async fn a_child_shuts_down_alone() {
        let parent = SessionScope::new("login", quiet());
        let call = parent.child("call");
        until_stopped(&call, "media");
        until_stopped(&parent, "presence");
        call.shutdown(Duration::from_secs(1)).await.unwrap();
        assert!(!parent.is_closed());
        assert_eq!(parent.len(), 1);
        parent.shutdown(Duration::from_secs(1)).await.unwrap();
    }

    #[tokio::test]
    async fn a_cooperative_task_runs_its_shutdown_procedure() {
        let scope = SessionScope::new("test", quiet());
        let cleaned = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&cleaned);
        scope
            .spawn_with_token("loop", move |stop| async move {
                stop.cancelled().await;
                tokio::task::yield_now().await; // cleanup that awaits
                flag.store(true, Ordering::Release);
            })
            .unwrap();
        scope.shutdown(Duration::from_secs(1)).await.unwrap();
        assert!(cleaned.load(Ordering::Acquire));
    }
}
