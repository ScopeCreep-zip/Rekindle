//! `OwnRoutes`: the node's private routes, one owner per route class
//! (plan C7.9a, D4 "own routes react to `RouteChange`"). One per node, like
//! `RouteImports`.
//!
//! veilid-core 0.5.8 (the pinned fork, ADR 0017), checked in source:
//! - `RouteChange.dead_routes` lists routes that died **or were released**
//!   (`veilid_api/types/veilid_state.rs:272-278`), so a dead route is forgotten, never
//!   released: releasing an id Veilid no longer knows is `InvalidArgument`,
//!   logged at ERROR (`veilid_api/api.rs:894-920`).
//! - Allocation returns `TryAgain` until PublicInternet peer info is
//!   published, with too few nodes, or when the new route fails its test
//!   (`route_allocate.rs:170-176,212`, `api.rs:831-839`). Only `TryAgain` is
//!   retried; any other error is a visible failure.
//! - A manual route has no lifetime: Veilid tests it and kills it on a
//!   failed test, and persists it across restarts. So a route that is
//!   replaced while live is released here, or it would live on unused.
//!
//! Allocation is single-flight per class and starts once the node is ready
//! (`public_internet_ready`), so the route exists before login and login
//! is reachable at once (owner, 2026-10-07: login and "Connected" are
//! paired). Logout releases the session's routes and allocates fresh ones
//! for the next login: a route is never shared by two identities, which
//! would link them.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use parking_lot::Mutex;
use rekindle_lifecycle::SessionScope;
use rekindle_records::port::{RouteClass, RouteState};
use tokio::sync::watch;

use crate::ordered_watch::{Follow, OrderedWatch};
use crate::route_death::{DeathCause, RouteDeath, TestTally};

/// Why an allocation did not produce a route.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AllocError {
    /// Veilid's `TryAgain`: retry after a backoff.
    TryAgain(String),
    /// Anything else: not retried.
    Fatal(String),
}

/// The calls `OwnRoutes` makes to the network (Veilid's in
/// `rekindle_protocol::own_routes`), behind a seam so its rules are tested
/// without a node.
#[async_trait]
pub trait RouteAllocator: Send + Sync + 'static {
    /// The route's id.
    type Id: Clone + PartialEq + std::fmt::Debug + Send + Sync + 'static;
    /// Allocate a route of `class`.
    async fn allocate(&self, class: RouteClass) -> Result<(Self::Id, Vec<u8>), AllocError>;
    /// Release a live route we own.
    fn release(&self, id: &Self::Id);
    /// The hop count a route of `class` is allocated with, when the
    /// network can say (for the death log, [`RouteDeath`]).
    fn hop_count(&self, class: RouteClass) -> Option<usize>;
}

/// First and longest wait between `TryAgain` retries.
const BACKOFF_MIN: Duration = Duration::from_millis(500);
const BACKOFF_MAX: Duration = Duration::from_secs(15);

/// A live route and when it landed.
struct Live<Id> {
    id: Id,
    blob: Vec<u8>,
    at: tokio::time::Instant,
}

struct Slot<Id> {
    class: RouteClass,
    wanted: AtomicBool,
    running: AtomicBool,
    /// A want or a death arrived while the task ran: run another pass.
    again: AtomicBool,
    current: Mutex<Option<Live<Id>>>,
    /// The live route's self-tests, for its death log.
    tally: Mutex<TestTally>,
    /// Routes [`OwnRoutes::replace`] took out of service that peers may
    /// still hold: kept allocated until [`OwnRoutes::release_retired`]
    /// (plan RH1r-c O4).
    retiring: Mutex<Vec<Live<Id>>>,
    /// The warm standby of a class that keeps one ([`keeps_standby`]):
    /// allocated, never handed to peers, promoted at once when the live
    /// route dies.
    standby: Mutex<Option<Live<Id>>>,
    /// Followed by the route publisher, the member-routes loop and each
    /// [`OwnRoutes::available`] wait, each through its own watch (RK1).
    state: OrderedWatch<RouteState>,
}

/// Whether `class` keeps a warm standby route (lead decision 60): the
/// `Sending` route is our safety route for every pinned send and call,
/// never handed to a peer, so a second one links nothing, and promoting it
/// on a death leaves no send gap. A standby is allocated like any manual
/// route, so Veilid tests it and marks it published, which keeps it out of
/// its safety-route pool (veilid-core 0.5.8 `veilid_api/api.rs:829-850`,
/// `route_select.rs:348-359`).
fn keeps_standby(class: RouteClass) -> bool {
    class == RouteClass::Sending
}

impl<Id> Slot<Id> {
    /// The live route is up and, for a class that keeps one, its standby.
    fn full(&self) -> bool {
        self.current.lock().is_some()
            && (!keeps_standby(self.class) || self.standby.lock().is_some())
    }

    /// Promote the standby into the live place, if there is one: its blob.
    fn promote(&self) -> Option<Vec<u8>> {
        let standby = self.standby.lock().take()?;
        let blob = standby.blob.clone();
        *self.tally.lock() = TestTally::default();
        *self.current.lock() = Some(Live {
            at: tokio::time::Instant::now(),
            ..standby
        });
        Some(blob)
    }

    /// The class's new state once its live route is gone: the promoted
    /// standby, or allocating.
    fn after_loss(&self) {
        match self.promote() {
            Some(blob) => {
                tracing::info!(class = ?self.class, "own standby route promoted");
                self.state.send_replace(RouteState::Available { blob });
            }
            None => {
                self.state.send_replace(RouteState::Allocating);
            }
        }
    }

    fn new(class: RouteClass) -> Self {
        Self {
            class,
            wanted: AtomicBool::new(false),
            running: AtomicBool::new(false),
            again: AtomicBool::new(false),
            current: Mutex::new(None),
            tally: Mutex::new(TestTally::default()),
            retiring: Mutex::new(Vec::new()),
            standby: Mutex::new(None),
            state: OrderedWatch::new(RouteState::Idle),
        }
    }
}

/// The node's own routes.
pub struct OwnRoutes<A: RouteAllocator> {
    alloc: A,
    /// Readiness, one watch per waiting class task (RK1).
    ready: Follow<bool>,
    scope: Arc<SessionScope>,
    backoff_min: Duration,
    general: Slot<A::Id>,
    media: Slot<A::Id>,
    sending: Slot<A::Id>,
}

/// Every class, in the order a pass over them takes.
const CLASSES: [RouteClass; 3] = [RouteClass::General, RouteClass::Media, RouteClass::Sending];

impl<A: RouteAllocator> std::fmt::Debug for OwnRoutes<A> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OwnRoutes")
            .field("general", &self.general.state.get())
            .field("media", &self.media.state.get())
            .field("sending", &self.sending.state.get())
            .finish_non_exhaustive()
    }
}

impl<A: RouteAllocator> OwnRoutes<A> {
    /// An owner over `alloc`, allocating once `ready` holds, on a scope of
    /// its own that lives as long as the node ([`shutdown`](Self::shutdown)).
    #[must_use]
    pub fn new(alloc: A, ready: Follow<bool>) -> Arc<Self> {
        Self::with_backoff(alloc, ready, BACKOFF_MIN)
    }

    fn with_backoff(alloc: A, ready: Follow<bool>, backoff_min: Duration) -> Arc<Self> {
        Arc::new(Self {
            alloc,
            ready,
            scope: SessionScope::new(
                "own routes",
                Arc::new(|task| tracing::error!(task, "own route task panicked")),
            ),
            backoff_min,
            general: Slot::new(RouteClass::General),
            media: Slot::new(RouteClass::Media),
            sending: Slot::new(RouteClass::Sending),
        })
    }

    fn slot(&self, class: RouteClass) -> &Slot<A::Id> {
        match class {
            RouteClass::General => &self.general,
            RouteClass::Media => &self.media,
            RouteClass::Sending => &self.sending,
        }
    }

    /// The class's state, to follow.
    #[must_use]
    pub fn state(&self, class: RouteClass) -> watch::Receiver<RouteState> {
        self.slot(class).state.subscribe()
    }

    /// The class's route blob once one is live: at once while one is, else
    /// when the next allocation lands. Callers bound the wait.
    pub async fn available(&self, class: RouteClass) -> Vec<u8> {
        let mut state = self.state(class);
        loop {
            if let RouteState::Available { blob } = &*state.borrow_and_update() {
                return blob.clone();
            }
            if state.changed().await.is_err() {
                return std::future::pending().await;
            }
        }
    }

    /// The class's live route blob, if any.
    #[must_use]
    pub fn blob(&self, class: RouteClass) -> Option<Vec<u8>> {
        self.slot(class)
            .current
            .lock()
            .as_ref()
            .map(|live| live.blob.clone())
    }

    /// The class's live route id, if any.
    #[must_use]
    pub fn id(&self, class: RouteClass) -> Option<A::Id> {
        self.slot(class)
            .current
            .lock()
            .as_ref()
            .map(|live| live.id.clone())
    }

    /// The class's warm standby route id, if it keeps one and it is up
    /// ([`keeps_standby`]): never handed to a peer, so a send over it as
    /// its safety route charges nothing to the live route (plan LF9c).
    #[must_use]
    pub fn standby_id(&self, class: RouteClass) -> Option<A::Id> {
        self.slot(class)
            .standby
            .lock()
            .as_ref()
            .map(|live| live.id.clone())
    }

    /// How long the class's live route has been up, if one is.
    #[must_use]
    pub fn age(&self, class: RouteClass) -> Option<Duration> {
        self.slot(class)
            .current
            .lock()
            .as_ref()
            .map(|live| live.at.elapsed())
    }

    /// Want a route of `class`: allocate one unless one is live or being
    /// allocated.
    pub fn want(self: &Arc<Self>, class: RouteClass) {
        self.slot(class).wanted.store(true, Ordering::SeqCst);
        self.start(class);
    }

    /// Veilid reported these routes dead (`RouteChange.dead_routes`): forget
    /// ours among them, without releasing, and allocate a replacement.
    pub fn on_dead(self: &Arc<Self>, dead: &[A::Id]) {
        for class in CLASSES {
            let slot = self.slot(class);
            // Veilid released a dead retiring route itself.
            slot.retiring.lock().retain(|live| !dead.contains(&live.id));
            // So did a dead standby: a new one is allocated.
            let standby_died = {
                let mut standby = slot.standby.lock();
                let hit = standby.as_ref().is_some_and(|live| dead.contains(&live.id));
                if hit {
                    standby.take();
                }
                hit
            };
            let died = {
                let mut current = slot.current.lock();
                let hit = current.as_ref().is_some_and(|live| dead.contains(&live.id));
                if hit {
                    current.take()
                } else {
                    None
                }
            };
            if let Some(old) = died {
                self.death(class, DeathCause::VeilidReported, &old).log();
                tracing::info!(?class, "own route died; allocating a replacement");
                slot.after_loss();
                self.start(class);
            } else if standby_died {
                self.start(class);
            }
        }
    }

    /// Replace our live route of `class`, which our own self-test found
    /// dead (plan RH1r-c O3) though Veilid has not reported it: a new one
    /// is allocated, and the old one stays allocated, out of service, while
    /// a peer may still send to it, until [`Self::release_retired`] (O4).
    pub fn replace(self: &Arc<Self>, class: RouteClass) {
        let slot = self.slot(class);
        let Some(old) = slot.current.lock().take() else {
            return;
        };
        self.death(class, DeathCause::SelfTest, &old).log();
        tracing::info!(
            ?class,
            "own route failed its self-test; allocating a replacement"
        );
        slot.retiring.lock().push(old);
        slot.after_loss();
        self.start(class);
    }

    /// A self-test of the class's live route came back `ok` (plan RH1r-c
    /// O1): kept for the route's death log.
    pub fn self_tested(&self, class: RouteClass, ok: bool) {
        let slot = self.slot(class);
        if slot.current.lock().is_some() {
            slot.tally.lock().record(tokio::time::Instant::now(), ok);
        }
    }

    /// The identifier-free record of `old`'s death (r7 design §4.6); the
    /// class's self-test tally starts over for its replacement.
    fn death(&self, class: RouteClass, cause: DeathCause, old: &Live<A::Id>) -> RouteDeath {
        let tally = std::mem::take(&mut *self.slot(class).tally.lock());
        RouteDeath::new(
            class,
            cause,
            self.alloc.hop_count(class),
            old.at,
            tally,
            *self.ready.follow().borrow(),
            tokio::time::Instant::now(),
        )
    }

    /// Release the routes of `class` that [`Self::replace`] retired, once
    /// their replacement is published (O4).
    pub fn release_retired(&self, class: RouteClass) {
        let retired: Vec<Live<A::Id>> = std::mem::take(&mut *self.slot(class).retiring.lock());
        for live in retired {
            self.alloc.release(&live.id);
        }
    }

    /// How many retired routes of `class` are still allocated.
    #[must_use]
    pub fn retiring(&self, class: RouteClass) -> usize {
        self.slot(class).retiring.lock().len()
    }

    /// Release every live route and stop wanting them (logout, exit). An
    /// allocation in flight releases its route when it lands.
    pub fn release_all(&self) {
        for class in CLASSES {
            self.release(class);
        }
    }

    /// Release the class's live route and stop wanting it. An allocation in
    /// flight releases its route when it lands.
    pub fn release(&self, class: RouteClass) {
        let slot = self.slot(class);
        slot.wanted.store(false, Ordering::SeqCst);
        if let Some(live) = slot.current.lock().take() {
            self.alloc.release(&live.id);
        }
        if let Some(live) = slot.standby.lock().take() {
            self.alloc.release(&live.id);
        }
        self.release_retired(class);
        slot.state.send_replace(RouteState::Idle);
    }

    /// Node shutdown (app exit): stop allocating and release every route.
    /// An allocation in flight is awaited, not cut, and its route released
    /// when it lands, so the caller may stop the network once this returns.
    pub async fn shutdown(&self) {
        self.scope.token().cancel();
        self.release_all();
        self.scope.close_and_wait().await;
    }

    /// Logout: release this session's routes, then allocate fresh ones for
    /// the next login, so no route is shared by two identities.
    pub fn renew(self: &Arc<Self>) {
        self.release_all();
        self.want_all();
    }

    /// Want the classes every node holds from node-ready: the general route
    /// and the media route (`Sending` is wanted by the session that sends
    /// over it, [`want`](Self::want)).
    pub fn want_all(self: &Arc<Self>) {
        self.want(RouteClass::General);
        self.want(RouteClass::Media);
    }

    /// Start the class's allocation task unless one runs, it is not
    /// wanted, or its routes are up (the live one and any standby).
    fn start(self: &Arc<Self>, class: RouteClass) {
        let slot = self.slot(class);
        if !slot.wanted.load(Ordering::SeqCst) || slot.full() {
            return;
        }
        if slot.running.swap(true, Ordering::SeqCst) {
            // The running task picks this up when its pass ends.
            slot.again.store(true, Ordering::SeqCst);
            return;
        }
        let this = Arc::clone(self);
        self.scope
            .spawn_with_token_or_drop("own route allocation", move |stop| async move {
                this.allocate_loop(class, stop).await;
            });
    }

    async fn allocate_loop(
        self: Arc<Self>,
        class: RouteClass,
        stop: tokio_util::sync::CancellationToken,
    ) {
        let slot = self.slot(class);
        loop {
            slot.again.store(false, Ordering::SeqCst);
            self.allocate_until_done(class, &stop).await;
            slot.running.store(false, Ordering::SeqCst);
            // Only a want or a death that landed during the pass runs another
            // (a failure for good waits for the next want).
            let again = slot.again.swap(false, Ordering::SeqCst)
                && !stop.is_cancelled()
                && slot.wanted.load(Ordering::SeqCst)
                && !slot.full()
                && !slot.running.swap(true, Ordering::SeqCst);
            if !again {
                return;
            }
        }
    }

    /// Allocate until the routes are up (the live one, then any standby),
    /// the class is not wanted, Veilid fails for good, or `stop`.
    async fn allocate_until_done(
        &self,
        class: RouteClass,
        stop: &tokio_util::sync::CancellationToken,
    ) {
        let slot = self.slot(class);
        let mut backoff = self.backoff_min;
        let mut ready = self.ready.follow();
        while slot.wanted.load(Ordering::SeqCst) && !slot.full() {
            // The class's state is its live route's: a standby's allocation
            // never shows.
            let live_up = slot.current.lock().is_some();
            if !live_up {
                slot.state.send_replace(RouteState::Allocating);
            }
            if stop
                .run_until_cancelled(ready.wait_for(|r| *r))
                .await
                .is_none_or(|r| r.is_err())
            {
                return;
            }
            // An allocation that started runs to its end, never cut at
            // `stop`: Veilid's `new_custom_private_route` holds a
            // `DurationRecorder` across the allocation and its test
            // (0.5.8 `veilid_api/api.rs:812-842`), and dropping it unfinished
            // logs ERROR "DurationRecorder dropped with no recording"
            // (`logging/duration_recorder.rs:84-90`; live A/B round 3
            // R3-6). Veilid bounds the call itself (the route test's RPC
            // timeout), so [`shutdown`](Self::shutdown) waits for it.
            let result = self.alloc.allocate(class).await;
            match result {
                Ok((id, blob)) => {
                    if stop.is_cancelled() || !slot.wanted.load(Ordering::SeqCst) {
                        // Released (logout) or shut down while allocating:
                        // not ours to keep.
                        self.alloc.release(&id);
                        return;
                    }
                    if slot.current.lock().is_some() {
                        let replaced = slot.standby.lock().replace(Live {
                            id,
                            blob,
                            at: tokio::time::Instant::now(),
                        });
                        if let Some(old) = replaced {
                            self.alloc.release(&old.id);
                        }
                        tracing::info!(class = ?slot.class, "own standby route allocated");
                        continue;
                    }
                    *slot.tally.lock() = TestTally::default();
                    let replaced = slot.current.lock().replace(Live {
                        id,
                        blob: blob.clone(),
                        at: tokio::time::Instant::now(),
                    });
                    if let Some(old) = replaced {
                        self.alloc.release(&old.id);
                    }
                    tracing::info!(class = ?slot.class, blob_len = blob.len(), "own route allocated");
                    slot.state.send_replace(RouteState::Available { blob });
                    backoff = self.backoff_min;
                }
                Err(AllocError::TryAgain(reason)) => {
                    tracing::debug!(?class, %reason, retry_ms = backoff.as_millis(), "own route: try again");
                    if !live_up {
                        slot.state
                            .send_replace(RouteState::Unavailable { retry_in: backoff });
                    }
                    if stop
                        .run_until_cancelled(tokio::time::sleep(backoff))
                        .await
                        .is_none()
                    {
                        return;
                    }
                    backoff = backoff.saturating_mul(2).min(BACKOFF_MAX);
                }
                Err(AllocError::Fatal(reason)) => {
                    tracing::warn!(?class, %reason, live_up, "own route allocation failed");
                    if !live_up {
                        slot.state.send_replace(RouteState::Failed { reason });
                    }
                    return;
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "own_routes_tests.rs"]
mod tests;
