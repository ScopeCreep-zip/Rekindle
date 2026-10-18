//! Private routes: peers' we import and send to, and our own.

use std::time::Duration;

use tokio::sync::watch;

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

/// A class of our own private route: `General` for messages, calls and the
/// published profile/mailbox route; `Media` for realtime media, relays
/// chosen for latency, never substituted by the general route (r6 R-NET2).
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

    /// Our route of `class`, to follow.
    fn own_state(&self, class: RouteClass) -> watch::Receiver<RouteState>;

    /// Our route of `class` once one is live; callers bound the wait.
    async fn available_own(&self, class: RouteClass) -> Vec<u8>;

    /// How long our live route of `class` has been up, if one is.
    fn own_age(&self, class: RouteClass) -> Option<Duration>;

    /// Want both route classes: allocate each unless live or allocating.
    fn want_own(&self);

    /// Our routes the network reported dead, by id text: forget them
    /// without releasing, and allocate replacements.
    fn on_dead_own(&self, dead: &[String]);

    /// Logout: release this session's routes, then allocate fresh ones, so
    /// no route is shared by two identities.
    fn renew_own(&self);

    /// Release our routes and stop wanting them (lock).
    fn release_own(&self);

    /// Node shutdown: stop allocating, release, and wait for allocations in
    /// flight.
    async fn shutdown_own(&self);
}
