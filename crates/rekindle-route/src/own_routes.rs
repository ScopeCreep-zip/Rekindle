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
use rekindle_records::port::{
    OwnDeaths, RouteBudgetState, RouteClass, RoutePauseCause, RouteState,
};
use tokio::sync::watch;

use crate::ordered_watch::{Follow, OrderedWatch};
use crate::own_route_accounts::RouteAccounts;
use crate::route_death::{DeathCause, RouteDeath};

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
    /// Routes [`OwnRoutes::replace`] took out of service that peers may
    /// still hold: kept allocated until [`OwnRoutes::release_retired`]
    /// (plan RH1r-c O4).
    retiring: Mutex<Vec<Live<Id>>>,
    /// Followed by the route publisher, the member-routes loop and each
    /// [`OwnRoutes::available`] wait, each through its own watch (RK1).
    state: OrderedWatch<RouteState>,
}

impl<Id> Slot<Id> {
    /// The live route is up.
    fn full(&self) -> bool {
        self.current.lock().is_some()
    }

    /// The class's new state once its live route is gone: allocating.
    fn after_loss(&self) {
        self.state.send_replace(RouteState::Allocating);
    }

    fn new(class: RouteClass) -> Self {
        Self {
            class,
            wanted: AtomicBool::new(false),
            running: AtomicBool::new(false),
            again: AtomicBool::new(false),
            current: Mutex::new(None),
            retiring: Mutex::new(Vec::new()),
            state: OrderedWatch::new(RouteState::Idle),
        }
    }
}

pub use rekindle_records::port::{SaveBudget, WallMs};

/// The node's own routes.
pub struct OwnRoutes<A: RouteAllocator> {
    /// The new-route budget, the pause and the death counts (plan RV1).
    accounts: RouteAccounts,
    alloc: A,
    /// Readiness, one watch per waiting class task (RK1).
    ready: Follow<bool>,
    scope: Arc<SessionScope>,
    backoff_min: Duration,
    general: Slot<A::Id>,
    media: Slot<A::Id>,
}

/// Every class, in the order a pass over them takes.
const CLASSES: [RouteClass; 2] = [RouteClass::General, RouteClass::Media];

impl<A: RouteAllocator> std::fmt::Debug for OwnRoutes<A> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OwnRoutes")
            .field("general", &self.general.state.get())
            .field("media", &self.media.state.get())
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
            accounts: RouteAccounts::new(),
            alloc,
            ready,
            scope: SessionScope::new(
                "own routes",
                Arc::new(|task| tracing::error!(task, "own route task panicked")),
            ),
            backoff_min,
            general: Slot::new(RouteClass::General),
            media: Slot::new(RouteClass::Media),
        })
    }

    fn slot(&self, class: RouteClass) -> &Slot<A::Id> {
        match class {
            RouteClass::General => &self.general,
            RouteClass::Media => &self.media,
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
            }
        }
    }

    /// Rotate our live route of `class`, which the evidence shows dead
    /// from outside though Veilid has not reported it (plan RV1): a new one
    /// is allocated (Veilid tests it at allocation, 0.5.8
    /// `veilid_api/api.rs:831`), and the old one stays allocated, out of
    /// service, while a peer may still send to it, until
    /// [`Self::release_retired`] (O4).
    pub fn replace(self: &Arc<Self>, class: RouteClass) {
        let slot = self.slot(class);
        let Some(old) = slot.current.lock().take() else {
            return;
        };
        self.death(class, DeathCause::Rotated, &old).log();
        tracing::info!(
            ?class,
            "own route rotated on evidence; allocating a replacement"
        );
        slot.retiring.lock().push(old);
        slot.after_loss();
        self.start(class);
    }

    /// The identifier-free record of `old`'s death (r7 design §4.6).
    fn death(&self, class: RouteClass, cause: DeathCause, old: &Live<A::Id>) -> RouteDeath {
        self.accounts.record_death(class, old.at.elapsed());
        RouteDeath::new(
            class,
            cause,
            self.alloc.hop_count(class),
            old.at,
            *self.ready.follow().borrow(),
            tokio::time::Instant::now(),
        )
    }

    /// Own-route deaths of `class` since the node started (Veilid's reports
    /// and our rotations), young ones counted apart.
    #[must_use]
    pub fn deaths(&self, class: RouteClass) -> OwnDeaths {
        self.accounts.deaths(class)
    }

    /// Why a class is down without a new route, if one is, to follow.
    #[must_use]
    pub fn paused(&self) -> watch::Receiver<Option<RoutePauseCause>> {
        self.accounts.paused()
    }

    /// Unlock: the identity's new-route budget `state`, read with `wall`
    /// and persisted through `save`. Each class's live route is about to be
    /// published, so each spends a token now; a class the budget cannot pay
    /// for is released and stays down.
    pub fn attach_budget(
        self: &Arc<Self>,
        state: RouteBudgetState,
        wall: WallMs,
        save: SaveBudget,
    ) {
        self.accounts.attach(state, wall, save);
        for class in CLASSES {
            if self.slot(class).current.lock().is_some() && !self.accounts.spend() {
                self.pause(class);
            }
        }
    }

    /// Unlock with a stored budget that could not be read: no class is
    /// paid for, so each live route is released before it is published and
    /// every class stays down, its cause `Unreadable` (fail-closed).
    pub fn attach_unreadable_budget(self: &Arc<Self>) {
        self.accounts.attach_unreadable();
        for class in CLASSES {
            self.pause(class);
        }
    }

    /// Lock: the identity's budget is no longer ours to spend.
    pub fn detach_budget(&self) {
        self.accounts.detach();
    }

    /// Spend a token for a route whose blob leaves the device but which we
    /// do not own here (a relay offer): `false` when the budget is spent or
    /// unreadable, or no identity is unlocked.
    pub fn spend_external(&self) -> bool {
        self.accounts.spend_external()
    }

    /// The class cannot be paid for: its live route (if any) is released,
    /// and it stays down (`Allocating`) with the pause shown.
    fn pause(&self, class: RouteClass) {
        let slot = self.slot(class);
        if let Some(live) = slot.current.lock().take() {
            self.alloc.release(&live.id);
        }
        slot.state.send_replace(RouteState::Allocating);
        let cause = self.accounts.mark_paused();
        tracing::warn!(
            ?class,
            ?cause,
            "no new route can be paid for; the class stays down"
        );
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
    /// and the media route.
    pub fn want_all(self: &Arc<Self>) {
        self.want(RouteClass::General);
        self.want(RouteClass::Media);
    }

    /// Start the class's allocation task unless one runs, it is not
    /// wanted, or its route is up.
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

    /// Allocate until the route is up,
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
            // Every new route spends from the budget once it lands (a
            // `TryAgain` spends nothing); an empty one leaves the class
            // down, never a silent redraw (plan RV1).
            if !self.accounts.can_spend() {
                self.pause(class);
                return;
            }
            let result = self.alloc.allocate(class).await;
            match result {
                Ok((id, blob)) => {
                    if stop.is_cancelled() || !slot.wanted.load(Ordering::SeqCst) {
                        // Released (logout) or shut down while allocating:
                        // not ours to keep.
                        self.alloc.release(&id);
                        return;
                    }
                    if !self.accounts.spend() {
                        self.alloc.release(&id);
                        self.pause(class);
                        return;
                    }
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
