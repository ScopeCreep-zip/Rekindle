//! Private routes: peers' we import and send to, and our own.

use std::time::Duration;

use tokio::sync::watch;

pub use rekindle_types::subscription_events::RoutePauseCause;

use super::PortError;

/// A peer's imported private route, by its route id text. Opaque: only a
/// route port reads it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PeerTarget(String);

impl PeerTarget {
    /// The target for a route id's text, as a route port prints it.
    #[must_use]
    pub fn new(route_id: String) -> Self {
        Self(route_id)
    }

    /// The route id's text, for the route port that issued it.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The unlocked identity's new-route budget as it is persisted (plan RV1,
/// owner decision 2026-11-22): the bucket `rekindle_route::budget` spends
/// from and the identity's database keeps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RouteBudgetState {
    /// Tokens left now.
    pub tokens: u32,
    /// Wall time (ms) of the last whole-day refill.
    pub refilled_ms: u64,
    /// Wall time (ms) the current week began.
    pub week_start_ms: u64,
    /// Tokens spent since `week_start_ms`.
    pub week_spent: u32,
}

/// Own-route deaths of one class since the node started (plan RV1).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OwnDeaths {
    /// Every death: the network's reports and our rotations on evidence.
    pub total: u32,
    /// Of those, the routes that died younger than
    /// `rekindle_route::budget::YOUNG_ROUTE_AGE`, counted apart.
    pub young: u32,
}

/// The wall clock (ms) the budget reads.
pub type WallMs = std::sync::Arc<dyn Fn() -> u64 + Send + Sync>;
/// Where each spend of the budget is persisted (the identity's database).
pub type SaveBudget = std::sync::Arc<dyn Fn(RouteBudgetState) + Send + Sync>;

/// A class of our own private route: `General` for messages, calls and the
/// published profile/mailbox route; `Media` for realtime media, relays
/// chosen for latency, never substituted by the general route (r6 R-NET2).
/// Both are receive routes; a send's safety route is Veilid's pool route,
/// never one of ours (plan RV1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RouteClass {
    General,
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
    /// The network said try again; the next attempt is in `retry_in`.
    Unavailable { retry_in: Duration },
    /// The network refused for good; the next want retries.
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

/// Peers' routes and our own. A send that fails with
/// [`PortError::RouteUnusable`] is the caller's to report through
/// [`invalidate_after_send_failure`](Self::invalidate_after_send_failure).
#[async_trait::async_trait]
pub trait RoutePort: Send + Sync {
    /// Import a peer's route blob (cached per blob).
    ///
    /// # Errors
    /// The blob does not decode or import.
    fn import(&self, blob: &[u8]) -> Result<PeerTarget, PortError>;

    /// Peers' routes died: forget them; the blobs they came from.
    fn on_dead_remote(&self, dead: &[PeerTarget]) -> Vec<Vec<u8>>;

    /// A send to `target` failed with no connection: forget it so the next
    /// send imports the peer's current blob.
    fn invalidate_after_send_failure(&self, target: &PeerTarget);

    /// Our live route of `class`, if any.
    fn own_blob(&self, class: RouteClass) -> Option<Vec<u8>>;

    /// Our live route of `class` by id text, as an inbound event's `route`
    /// names it.
    fn own_id(&self, class: RouteClass) -> Option<String>;

    /// Our route of `class`, to follow.
    fn own_state(&self, class: RouteClass) -> watch::Receiver<RouteState>;

    /// Our route of `class` once one is live; callers bound the wait.
    async fn available_own(&self, class: RouteClass) -> Vec<u8>;

    /// How long our live route of `class` has been up, if one is.
    fn own_age(&self, class: RouteClass) -> Option<Duration>;

    /// Want the General and Media classes: allocate each unless live or
    /// allocating.
    fn want_own(&self);

    /// Want our route of `class`: allocate it unless live or allocating.
    fn want_own_class(&self, class: RouteClass);

    /// Release our route of `class` and stop wanting it.
    fn release_own_class(&self, class: RouteClass);

    /// Rotate our live route of `class`, which the evidence shows dead
    /// from outside though the network has not said so (plan RV1: inbound
    /// silence while a peer is online, or a peer's `'L'` report): allocate
    /// a new one, which Veilid tests at allocation (0.5.8
    /// `veilid_api/api.rs:831`), and keep the old one allocated, out of
    /// service, until [`release_retired_own`](Self::release_retired_own).
    fn replace_own_class(&self, class: RouteClass);

    /// Release the routes of `class` that a replacement retired, once the
    /// replacement is published (O4).
    fn release_retired_own(&self, class: RouteClass);

    /// Our routes the network reported dead, by id text: forget them
    /// without releasing, and allocate replacements.
    fn on_dead_own(&self, dead: &[String]);

    /// Logout: release this session's routes, then allocate fresh ones, so
    /// no route is shared by two identities.
    fn renew_own(&self);

    /// Release our routes of every class and stop wanting them (lock).
    fn release_own(&self);

    /// Node shutdown: stop allocating, release, and wait for allocations in
    /// flight.
    async fn shutdown_own(&self);

    /// Unlock: the identity's new-route budget `state`, read with `wall`
    /// and persisted through `save` (plan RV1). Each live class is about to
    /// be published and spends a token now; one the budget cannot pay for
    /// stays down.
    fn attach_budget(&self, state: RouteBudgetState, wall: WallMs, save: SaveBudget);

    /// Unlock with a stored budget that could not be read: nothing is paid
    /// for, so every class stays down with the pause's cause
    /// `Unreadable` (fail-closed, never a full budget in its place).
    fn attach_unreadable_budget(&self);

    /// Lock: the identity's budget is no longer ours to spend.
    fn detach_budget(&self);

    /// Spend a token for a route whose blob leaves the device but which the
    /// own-route owner does not hold (a relay offer): `false` when the
    /// budget is spent or no identity is unlocked.
    fn spend_budget(&self) -> bool;

    /// Why a class is down with no new route, if one is, to follow.
    fn own_paused(&self) -> watch::Receiver<Option<RoutePauseCause>>;

    /// Own-route deaths of `class` since the node started (the network's
    /// reports and our rotations on evidence), young ones counted apart.
    fn own_deaths(&self, class: RouteClass) -> OwnDeaths;
}
