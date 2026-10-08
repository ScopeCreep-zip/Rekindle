//! `OwnRoutes`: the node's private routes, one owner per route class
//! (plan C7.9a, D4 "own routes react to `RouteChange`"). One per node, like
//! `RouteImports`.
//!
//! veilid-core 0.5.7, checked in source:
//! - `RouteChange.dead_routes` lists routes that died **or were released**
//!   (`veilid_state.rs:238-245`), so a dead route is forgotten, never
//!   released: releasing an id Veilid no longer knows is `InvalidArgument`,
//!   logged at ERROR (`api.rs:516-531`).
//! - Allocation returns `TryAgain` until PublicInternet peer info is
//!   published, with too few nodes, or when the new route fails its test
//!   (`route_allocate.rs:139-145`, `api.rs:447-457`). Only `TryAgain` is
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
use tokio::sync::watch;

/// A class of our own private route.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RouteClass {
    /// Messages, calls and the published profile/mailbox route.
    General,
    /// Realtime media: relays chosen for latency (`Stability::LowLatency`,
    /// `Sequencing::PreferUnordered`), never substituted by the general
    /// route (r6 R-NET2).
    Media,
}

/// A class's route, as hosts see it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RouteState {
    /// Not wanted (logged out between releases, or shut down).
    Idle,
    /// Waiting for the network, or allocating.
    Allocating,
    /// A route is live: this blob is what peers import.
    Available { blob: Vec<u8> },
    /// Veilid said try again; the next attempt is in `retry_in`.
    Unavailable { retry_in: Duration },
    /// Veilid refused for good (not `TryAgain`); the next want retries.
    Failed { reason: String },
}

impl RouteState {
    /// The state without its payload, as a status indicator shows it.
    #[must_use]
    pub fn availability(&self) -> rekindle_types::subscription_events::RouteAvailability {
        use rekindle_types::subscription_events::RouteAvailability;
        match self {
            Self::Idle => RouteAvailability::Idle,
            Self::Allocating => RouteAvailability::Allocating,
            Self::Available { .. } => RouteAvailability::Available,
            Self::Unavailable { .. } => RouteAvailability::Unavailable,
            Self::Failed { .. } => RouteAvailability::Failed,
        }
    }
}

/// Why an allocation did not produce a route.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AllocError {
    /// Veilid's `TryAgain`: retry after a backoff.
    TryAgain(String),
    /// Anything else: not retried.
    Fatal(String),
}

/// The Veilid calls `OwnRoutes` makes, behind a seam so its rules are
/// tested without a node.
#[async_trait]
pub trait RouteAllocator: Send + Sync + 'static {
    /// The route's id.
    type Id: Clone + PartialEq + std::fmt::Debug + Send + Sync + 'static;
    /// Allocate a route of `class`.
    async fn allocate(&self, class: RouteClass) -> Result<(Self::Id, Vec<u8>), AllocError>;
    /// Release a live route we own.
    fn release(&self, id: &Self::Id);
}

/// First and longest wait between `TryAgain` retries.
const BACKOFF_MIN: Duration = Duration::from_millis(500);
const BACKOFF_MAX: Duration = Duration::from_secs(15);

/// A live route and when it landed.
struct Live<Id> {
    id: Id,
    blob: Vec<u8>,
    at: std::time::Instant,
}

struct Slot<Id> {
    class: RouteClass,
    wanted: AtomicBool,
    running: AtomicBool,
    /// A want or a death arrived while the task ran: run another pass.
    again: AtomicBool,
    current: Mutex<Option<Live<Id>>>,
    state: watch::Sender<RouteState>,
}

impl<Id> Slot<Id> {
    fn new(class: RouteClass) -> Self {
        Self {
            class,
            wanted: AtomicBool::new(false),
            running: AtomicBool::new(false),
            again: AtomicBool::new(false),
            current: Mutex::new(None),
            state: watch::channel(RouteState::Idle).0,
        }
    }
}

/// The node's own routes.
pub struct OwnRoutes<A: RouteAllocator> {
    alloc: A,
    ready: watch::Receiver<bool>,
    scope: Arc<SessionScope>,
    backoff_min: Duration,
    general: Slot<A::Id>,
    media: Slot<A::Id>,
}

impl<A: RouteAllocator> std::fmt::Debug for OwnRoutes<A> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OwnRoutes")
            .field("general", &*self.general.state.borrow())
            .field("media", &*self.media.state.borrow())
            .finish_non_exhaustive()
    }
}

impl<A: RouteAllocator> OwnRoutes<A> {
    /// An owner over `alloc`, allocating once `ready` holds, on a scope of
    /// its own that lives as long as the node ([`shutdown`](Self::shutdown)).
    #[must_use]
    pub fn new(alloc: A, ready: watch::Receiver<bool>) -> Arc<Self> {
        Self::with_backoff(alloc, ready, BACKOFF_MIN)
    }

    fn with_backoff(alloc: A, ready: watch::Receiver<bool>, backoff_min: Duration) -> Arc<Self> {
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
        for class in [RouteClass::General, RouteClass::Media] {
            let slot = self.slot(class);
            let died = {
                let mut current = slot.current.lock();
                let hit = current.as_ref().is_some_and(|live| dead.contains(&live.id));
                if hit {
                    current.take();
                }
                hit
            };
            if died {
                tracing::info!(?class, "own route died; allocating a replacement");
                slot.state.send_replace(RouteState::Allocating);
                self.start(class);
            }
        }
    }

    /// Release every live route and stop wanting them (logout, exit). An
    /// allocation in flight releases its route when it lands.
    pub fn release_all(&self) {
        for class in [RouteClass::General, RouteClass::Media] {
            let slot = self.slot(class);
            slot.wanted.store(false, Ordering::SeqCst);
            if let Some(live) = slot.current.lock().take() {
                self.alloc.release(&live.id);
            }
            slot.state.send_replace(RouteState::Idle);
        }
    }

    /// Node shutdown (app exit): stop allocating and release every route.
    pub async fn shutdown(&self) {
        self.scope.token().cancel();
        self.release_all();
        self.scope.close_and_wait().await;
    }

    /// Logout: release this session's routes, then allocate fresh ones for
    /// the next login, so no route is shared by two identities.
    pub fn renew(self: &Arc<Self>) {
        self.release_all();
        self.want(RouteClass::General);
        self.want(RouteClass::Media);
    }

    /// Start the class's allocation task unless one runs, it is not
    /// wanted, or a route is live.
    fn start(self: &Arc<Self>, class: RouteClass) {
        let slot = self.slot(class);
        if !slot.wanted.load(Ordering::SeqCst) || slot.current.lock().is_some() {
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
                && slot.current.lock().is_none()
                && !slot.running.swap(true, Ordering::SeqCst);
            if !again {
                return;
            }
        }
    }

    /// Allocate until a route is live, the class is not wanted, Veilid
    /// fails for good, or `stop`.
    async fn allocate_until_done(
        &self,
        class: RouteClass,
        stop: &tokio_util::sync::CancellationToken,
    ) {
        let slot = self.slot(class);
        let mut backoff = self.backoff_min;
        let mut ready = self.ready.clone();
        while slot.wanted.load(Ordering::SeqCst) && slot.current.lock().is_none() {
            slot.state.send_replace(RouteState::Allocating);
            if stop
                .run_until_cancelled(ready.wait_for(|r| *r))
                .await
                .is_none_or(|r| r.is_err())
            {
                return;
            }
            let Some(result) = stop.run_until_cancelled(self.alloc.allocate(class)).await else {
                return;
            };
            match result {
                Ok((id, blob)) => {
                    if !slot.wanted.load(Ordering::SeqCst) {
                        // Released (logout) while allocating: not ours to keep.
                        self.alloc.release(&id);
                        return;
                    }
                    let replaced = slot.current.lock().replace(Live {
                        id,
                        blob: blob.clone(),
                        at: std::time::Instant::now(),
                    });
                    if let Some(old) = replaced {
                        self.alloc.release(&old.id);
                    }
                    tracing::info!(class = ?slot.class, blob_len = blob.len(), "own route allocated");
                    slot.state.send_replace(RouteState::Available { blob });
                    return;
                }
                Err(AllocError::TryAgain(reason)) => {
                    tracing::debug!(?class, %reason, retry_ms = backoff.as_millis(), "own route: try again");
                    slot.state
                        .send_replace(RouteState::Unavailable { retry_in: backoff });
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
                    tracing::warn!(?class, %reason, "own route allocation failed");
                    slot.state.send_replace(RouteState::Failed { reason });
                    return;
                }
            }
        }
    }
}

/// The Veilid allocator: `new_private_route` for General, a LowLatency /
/// PreferUnordered custom route for Media (same hop count, so anonymity is
/// unchanged; only relay selection differs).
pub struct VeilidRouteAllocator {
    api: veilid_core::VeilidAPI,
}

impl VeilidRouteAllocator {
    #[must_use]
    pub fn new(api: veilid_core::VeilidAPI) -> Self {
        Self { api }
    }
}

#[async_trait]
impl RouteAllocator for VeilidRouteAllocator {
    type Id = veilid_core::RouteId;

    async fn allocate(&self, class: RouteClass) -> Result<(Self::Id, Vec<u8>), AllocError> {
        let result = match class {
            RouteClass::General => self.api.new_private_route().await,
            RouteClass::Media => {
                self.api
                    .new_custom_private_route(veilid_core::PrivateSpec {
                        // Empty = all available crypto kinds, as the default does.
                        crypto_kinds: Vec::new(),
                        // 0 = the configured default hop count, as General.
                        hop_count: 0,
                        stability: veilid_core::Stability::LowLatency,
                        sequencing: veilid_core::Sequencing::PreferUnordered,
                    })
                    .await
            }
        };
        match result {
            Ok(route) => Ok((route.route_id, route.blob)),
            Err(veilid_core::VeilidAPIError::TryAgain { message }) => {
                Err(AllocError::TryAgain(message))
            }
            Err(e) => Err(AllocError::Fatal(e.to_string())),
        }
    }

    fn release(&self, id: &Self::Id) {
        if let Err(e) = self.api.release_private_route(id.clone()) {
            tracing::debug!(route = %id, error = %e, "releasing own route failed");
        }
    }
}

#[cfg(test)]
#[path = "own_routes_tests.rs"]
mod tests;
